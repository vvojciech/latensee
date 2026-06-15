use std::mem::MaybeUninit;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use pnet::packet::icmp::echo_request::MutableEchoRequestPacket;
use pnet::packet::icmp::{checksum, IcmpCode, IcmpPacket, IcmpTypes};

use crate::probe::socket;
use crate::trace::state::ProbeResult;

/// Classification of an ICMP response.
#[derive(Debug, PartialEq)]
pub enum IcmpResponseType {
    EchoReply,
    TimeExceeded(IpAddr),
}

/// Build an ICMP echo request packet.
///
/// Returns the raw bytes ready to send on a raw socket.
/// `size` is the total ICMP payload size (excluding the 8-byte ICMP header).
pub fn build_echo_request(identifier: u16, seq: u16, size: u16) -> Vec<u8> {
    let total_len = 8 + size as usize; // ICMP header (8) + payload
    let mut buf = vec![0u8; total_len];

    let mut packet = MutableEchoRequestPacket::new(&mut buf)
        .expect("buffer too small for ICMP echo request");

    packet.set_icmp_type(IcmpTypes::EchoRequest);
    packet.set_icmp_code(IcmpCode::new(0));
    packet.set_identifier(identifier);
    packet.set_sequence_number(seq);

    // Fill payload with sequential bytes for identification
    let payload_bytes: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    packet.set_payload(&payload_bytes);

    // Calculate and set checksum
    let icmp_packet = IcmpPacket::new(&buf).expect("failed to parse for checksum");
    let cksum = checksum(&icmp_packet);

    let mut packet = MutableEchoRequestPacket::new(&mut buf)
        .expect("buffer too small for ICMP echo request");
    packet.set_checksum(cksum);

    buf
}

/// Parse a received ICMP response buffer.
///
/// For echo replies, checks identifier and sequence match.
/// For time-exceeded, the source IP of the reply is the intermediate router --
/// extracted from the outer IP header (first 20 bytes of `buf` on raw sockets).
///
/// `buf` is the raw bytes received from the socket, starting at the IP header.
pub fn parse_icmp_response(
    buf: &[u8],
    identifier: u16,
    seq: u16,
) -> Option<IcmpResponseType> {
    // Raw ICMP socket on macOS/Linux includes the IP header (20 bytes min)
    if buf.len() < 28 {
        return None; // Too short for IP header + ICMP header
    }

    let ip_header_len = ((buf[0] & 0x0F) as usize) * 4;
    if buf.len() < ip_header_len + 8 {
        return None;
    }

    let icmp_buf = &buf[ip_header_len..];
    let icmp_type = icmp_buf[0];
    let _icmp_code = icmp_buf[1];

    match icmp_type {
        // Echo Reply (type 0)
        0 => {
            if icmp_buf.len() < 8 {
                return None;
            }
            let reply_id = u16::from_be_bytes([icmp_buf[4], icmp_buf[5]]);
            let reply_seq = u16::from_be_bytes([icmp_buf[6], icmp_buf[7]]);
            if reply_id == identifier && reply_seq == seq {
                Some(IcmpResponseType::EchoReply)
            } else {
                None
            }
        }
        // Time Exceeded (type 11)
        11 => {
            // Time-exceeded payload: original IP header (20 bytes) + first 8 bytes of original ICMP
            if icmp_buf.len() < 8 + 20 + 8 {
                return None;
            }
            let inner_icmp = &icmp_buf[8 + 20..]; // skip ICMP header + inner IP header
            let orig_id = u16::from_be_bytes([inner_icmp[4], inner_icmp[5]]);
            let orig_seq = u16::from_be_bytes([inner_icmp[6], inner_icmp[7]]);

            if orig_id == identifier && orig_seq == seq {
                // Source IP is the router that sent time-exceeded, from outer IP header
                let src_ip = IpAddr::V4(std::net::Ipv4Addr::new(
                    buf[12], buf[13], buf[14], buf[15],
                ));
                Some(IcmpResponseType::TimeExceeded(src_ip))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Send a single ICMP probe and wait for a response.
pub async fn send_probe(
    target: IpAddr,
    ttl: u8,
    seq: u16,
    timeout: Duration,
    size: u16,
    identifier: u16,
) -> ProbeResult {
    let timestamp = Instant::now();

    let result = tokio::task::spawn_blocking(move || {
        let ipv6 = target.is_ipv6();
        let sock = socket::create_icmp_socket(ipv6)?;
        socket::set_ttl(&sock, ttl, ipv6)?;
        socket::set_timeout(&sock, timeout)?;

        let packet = build_echo_request(identifier, seq, size);

        let dest: socket2::SockAddr = std::net::SocketAddr::new(target, 0).into();
        let send_time = Instant::now();
        sock.send_to(&packet, &dest)?;

        let mut recv_buf = [MaybeUninit::<u8>::uninit(); 1500];
        loop {
            let elapsed = send_time.elapsed();
            if elapsed >= timeout {
                return Ok::<Option<Duration>, std::io::Error>(None);
            }

            match sock.recv_from(&mut recv_buf) {
                Ok((n, _addr)) => {
                    let rtt = send_time.elapsed();
                    // SAFETY: recv_from guarantees the first n bytes are initialized
                    let received: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            recv_buf.as_ptr() as *const u8,
                            n,
                        )
                    };
                    if let Some(_response) =
                        parse_icmp_response(received, identifier, seq)
                    {
                        return Ok(Some(rtt));
                    }
                    // Not our packet, keep waiting
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return Ok(None); // Timeout
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
    })
    .await;

    let rtt = match result {
        Ok(Ok(rtt)) => rtt,
        _ => None,
    };

    ProbeResult {
        seq: seq as u64,
        rtt,
        timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn build_echo_request_produces_valid_packet() {
        let packet = build_echo_request(0x1234, 1, 0);

        // Minimum ICMP header is 8 bytes
        assert_eq!(packet.len(), 8);

        // Type 8 (echo request), code 0
        assert_eq!(packet[0], 8);
        assert_eq!(packet[1], 0);

        // Identifier
        assert_eq!(u16::from_be_bytes([packet[4], packet[5]]), 0x1234);

        // Sequence number
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 1);

        // Verify checksum: recompute and compare
        let icmp = IcmpPacket::new(&packet).unwrap();
        let expected_cksum = checksum(&icmp);
        let actual_cksum = u16::from_be_bytes([packet[2], packet[3]]);
        assert_eq!(actual_cksum, expected_cksum);
    }

    #[test]
    fn build_echo_request_with_payload_has_correct_length() {
        let sizes: &[u16] = &[0, 32, 56, 128, 1024];
        for &size in sizes {
            let packet = build_echo_request(0xABCD, 5, size);
            assert_eq!(
                packet.len(),
                8 + size as usize,
                "wrong length for payload size {}",
                size
            );
        }
    }

    #[test]
    fn parse_icmp_response_identifies_echo_reply() {
        // Construct a fake raw buffer: IP header (20 bytes) + ICMP echo reply
        let mut buf = vec![0u8; 28];

        // Minimal IPv4 header: version 4, IHL 5 (20 bytes)
        buf[0] = 0x45;

        // ICMP echo reply: type 0, code 0
        buf[20] = 0; // type
        buf[21] = 0; // code
        // checksum placeholder (not validated in parse)
        buf[22] = 0;
        buf[23] = 0;
        // identifier = 0x1234
        buf[24] = 0x12;
        buf[25] = 0x34;
        // sequence = 7
        buf[26] = 0x00;
        buf[27] = 0x07;

        let result = parse_icmp_response(&buf, 0x1234, 7);
        assert_eq!(result, Some(IcmpResponseType::EchoReply));
    }

    #[test]
    fn parse_icmp_response_identifies_time_exceeded() {
        // IP header (20) + ICMP time-exceeded header (8) + inner IP header (20) + inner ICMP (8)
        let mut buf = vec![0u8; 56];

        // Outer IPv4 header
        buf[0] = 0x45;
        // Source IP of the router: 10.0.0.1
        buf[12] = 10;
        buf[13] = 0;
        buf[14] = 0;
        buf[15] = 1;

        // ICMP time-exceeded: type 11, code 0
        buf[20] = 11;
        buf[21] = 0;

        // Inner IP header starts at offset 28 (20 + 8)
        buf[28] = 0x45; // version + IHL

        // Inner ICMP starts at offset 48 (28 + 20)
        buf[48] = 8; // original echo request type
        buf[49] = 0; // code
        // checksum
        buf[50] = 0;
        buf[51] = 0;
        // identifier = 0xBEEF
        buf[52] = 0xBE;
        buf[53] = 0xEF;
        // sequence = 42
        buf[54] = 0x00;
        buf[55] = 42;

        let result = parse_icmp_response(&buf, 0xBEEF, 42);
        assert_eq!(
            result,
            Some(IcmpResponseType::TimeExceeded(IpAddr::V4(
                Ipv4Addr::new(10, 0, 0, 1)
            )))
        );
    }

    #[test]
    fn parse_icmp_response_returns_none_for_unrelated_packets() {
        // Echo reply with wrong identifier
        let mut buf = vec![0u8; 28];
        buf[0] = 0x45;
        buf[20] = 0; // echo reply
        buf[24] = 0xFF; // wrong identifier
        buf[25] = 0xFF;
        buf[26] = 0x00;
        buf[27] = 0x01;

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1), None);

        // Wrong sequence
        buf[24] = 0x12;
        buf[25] = 0x34;
        buf[26] = 0x00;
        buf[27] = 0x99; // wrong seq

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1), None);

        // Unknown ICMP type (e.g. type 3 = destination unreachable)
        buf[20] = 3;
        assert_eq!(parse_icmp_response(&buf, 0x1234, 1), None);

        // Buffer too short
        assert_eq!(parse_icmp_response(&[0u8; 10], 0x1234, 1), None);
    }

    #[tokio::test]
    #[ignore] // Requires root privileges for raw ICMP sockets
    async fn send_probe_to_localhost_returns_rtt() {
        let result = send_probe(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            64,
            1,
            Duration::from_secs(2),
            32,
            0x7070, // "pp" for latensee
        )
        .await;

        assert_eq!(result.seq, 1);
        assert!(
            result.rtt.is_some(),
            "expected RTT for localhost probe, got None"
        );
        assert!(
            result.rtt.unwrap() < Duration::from_millis(100),
            "localhost RTT should be under 100ms, got {:?}",
            result.rtt.unwrap()
        );
    }
}
