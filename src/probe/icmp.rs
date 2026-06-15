use std::mem::MaybeUninit;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use pnet::packet::icmp::echo_request::MutableEchoRequestPacket;
use pnet::packet::icmp::{checksum, IcmpCode, IcmpPacket, IcmpType, IcmpTypes};

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
///
/// For IPv4: type 8, with computed checksum.
/// For IPv6: type 128 (ICMPv6 echo request), checksum left at 0 because
/// the kernel computes the ICMPv6 pseudo-header checksum automatically.
pub fn build_echo_request(identifier: u16, seq: u16, size: u16, ipv6: bool) -> Vec<u8> {
    let total_len = 8 + size as usize; // ICMP header (8) + payload
    let mut buf = vec![0u8; total_len];

    let mut packet = MutableEchoRequestPacket::new(&mut buf)
        .expect("buffer too small for ICMP echo request");

    if ipv6 {
        // ICMPv6 echo request: type 128, code 0
        packet.set_icmp_type(IcmpType::new(128));
    } else {
        packet.set_icmp_type(IcmpTypes::EchoRequest);
    }
    packet.set_icmp_code(IcmpCode::new(0));
    packet.set_identifier(identifier);
    packet.set_sequence_number(seq);

    // Fill payload with sequential bytes for identification
    let payload_bytes: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    packet.set_payload(&payload_bytes);

    if !ipv6 {
        // ICMPv4: compute checksum ourselves
        let icmp_packet = IcmpPacket::new(&buf).expect("failed to parse for checksum");
        let cksum = checksum(&icmp_packet);

        let mut packet = MutableEchoRequestPacket::new(&mut buf)
            .expect("buffer too small for ICMP echo request");
        packet.set_checksum(cksum);
    }
    // ICMPv6: leave checksum as 0; the kernel fills it using the pseudo-header

    buf
}

/// Parse a received ICMP response buffer.
///
/// For echo replies, checks identifier and sequence match.
/// For time-exceeded, the source IP of the reply is the intermediate router --
/// extracted from the outer IP header (first 20 bytes of `buf` on raw sockets).
///
/// **IPv4:** `buf` starts at the IP header (raw ICMP sockets include it).
/// **IPv6:** `buf` starts directly at the ICMPv6 header (the kernel strips IPv6 headers).
///
/// For IPv6 time-exceeded, the router IP is extracted from `recv_from()` by the caller
/// and passed via `router_addr`. For IPv4, it's read from the outer IP header in the buffer.
pub fn parse_icmp_response(
    buf: &[u8],
    identifier: u16,
    seq: u16,
    ipv6: bool,
) -> Option<IcmpResponseType> {
    if ipv6 {
        parse_icmpv6_response(buf, identifier, seq)
    } else {
        parse_icmpv4_response(buf, identifier, seq)
    }
}

/// Parse an ICMPv4 response (buffer includes IPv4 header).
fn parse_icmpv4_response(
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

/// Parse an ICMPv6 response (buffer starts at ICMPv6 header, no IPv6 header).
///
/// ICMPv6 type codes differ from ICMPv4:
/// - Echo Reply: type 129 (vs 0)
/// - Time Exceeded: type 3 (vs 11)
///
/// For time-exceeded, the router's address comes from `recvfrom()`, not the buffer.
/// We return a placeholder `Ipv6Addr::UNSPECIFIED` here; the caller substitutes the
/// real address from the socket.
fn parse_icmpv6_response(
    buf: &[u8],
    identifier: u16,
    seq: u16,
) -> Option<IcmpResponseType> {
    if buf.len() < 8 {
        return None;
    }

    let icmp_type = buf[0];

    match icmp_type {
        // ICMPv6 Echo Reply (type 129)
        129 => {
            let reply_id = u16::from_be_bytes([buf[4], buf[5]]);
            let reply_seq = u16::from_be_bytes([buf[6], buf[7]]);
            if reply_id == identifier && reply_seq == seq {
                Some(IcmpResponseType::EchoReply)
            } else {
                None
            }
        }
        // ICMPv6 Time Exceeded (type 3)
        3 => {
            // Payload: inner IPv6 header (40 bytes) + first 8 bytes of original ICMPv6
            if buf.len() < 8 + 40 + 8 {
                return None;
            }
            let inner_icmpv6 = &buf[8 + 40..]; // skip ICMPv6 header + inner IPv6 header
            let orig_id = u16::from_be_bytes([inner_icmpv6[4], inner_icmpv6[5]]);
            let orig_seq = u16::from_be_bytes([inner_icmpv6[6], inner_icmpv6[7]]);

            if orig_id == identifier && orig_seq == seq {
                // Router IP comes from recvfrom() addr, not from the buffer.
                // Use unspecified as placeholder; send_probe overwrites from socket addr.
                let src_ip = IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
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

        let packet = build_echo_request(identifier, seq, size, ipv6);

        let dest: socket2::SockAddr = std::net::SocketAddr::new(target, 0).into();
        let send_time = Instant::now();
        sock.send_to(&packet, &dest)?;

        let mut recv_buf = [MaybeUninit::<u8>::uninit(); 1500];
        loop {
            let elapsed = send_time.elapsed();
            if elapsed >= timeout {
                return Ok::<Option<(Duration, IpAddr)>, std::io::Error>(None);
            }

            match sock.recv_from(&mut recv_buf) {
                Ok((n, peer_addr)) => {
                    let rtt = send_time.elapsed();
                    // SAFETY: recv_from guarantees the first n bytes are initialized
                    let received: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            recv_buf.as_ptr() as *const u8,
                            n,
                        )
                    };
                    if let Some(response) =
                        parse_icmp_response(received, identifier, seq, ipv6)
                    {
                        let hop_addr = match response {
                            IcmpResponseType::EchoReply => target,
                            IcmpResponseType::TimeExceeded(addr) => {
                                if addr.is_unspecified() {
                                    peer_addr.as_socket().map(|s| s.ip()).unwrap_or(addr)
                                } else {
                                    addr
                                }
                            }
                        };
                        return Ok(Some((rtt, hop_addr)));
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

    let (rtt, addr) = match result {
        Ok(Ok(Some((rtt, addr)))) => (Some(rtt), Some(addr)),
        _ => (None, None),
    };

    ProbeResult {
        seq: seq as u64,
        rtt,
        timestamp,
        addr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn build_echo_request_v4_produces_valid_packet() {
        let packet = build_echo_request(0x1234, 1, 0, false);

        // Minimum ICMP header is 8 bytes
        assert_eq!(packet.len(), 8);

        // Type 8 (ICMPv4 echo request), code 0
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
    fn build_echo_request_v6_uses_type_128() {
        let packet = build_echo_request(0x1234, 1, 0, true);

        assert_eq!(packet.len(), 8);

        // Type 128 (ICMPv6 echo request), code 0
        assert_eq!(packet[0], 128);
        assert_eq!(packet[1], 0);

        // Identifier and sequence still present
        assert_eq!(u16::from_be_bytes([packet[4], packet[5]]), 0x1234);
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 1);

        // ICMPv6 checksum is computed by the kernel, so we leave it zero
        let cksum = u16::from_be_bytes([packet[2], packet[3]]);
        assert_eq!(cksum, 0, "ICMPv6 checksum should be 0 (kernel computes it)");
    }

    #[test]
    fn build_echo_request_v6_with_payload_has_correct_length() {
        let sizes: &[u16] = &[0, 32, 56, 128];
        for &size in sizes {
            let packet = build_echo_request(0xABCD, 5, size, true);
            assert_eq!(
                packet.len(),
                8 + size as usize,
                "wrong length for IPv6 payload size {}",
                size
            );
            // Type must still be 128
            assert_eq!(packet[0], 128);
        }
    }

    #[test]
    fn build_echo_request_with_payload_has_correct_length() {
        let sizes: &[u16] = &[0, 32, 56, 128, 1024];
        for &size in sizes {
            let packet = build_echo_request(0xABCD, 5, size, false);
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

        let result = parse_icmp_response(&buf, 0x1234, 7, false);
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

        let result = parse_icmp_response(&buf, 0xBEEF, 42, false);
        assert_eq!(
            result,
            Some(IcmpResponseType::TimeExceeded(IpAddr::V4(
                Ipv4Addr::new(10, 0, 0, 1)
            )))
        );
    }

    // --- ICMPv6 parsing tests ---
    // ICMPv6 raw sockets do NOT include the IPv6 header in received data.
    // Buffer starts directly at the ICMPv6 header.

    #[test]
    fn parse_icmpv6_response_identifies_echo_reply() {
        // ICMPv6 echo reply: type 129, code 0, then id + seq
        let mut buf = vec![0u8; 8];
        buf[0] = 129; // type
        buf[1] = 0;   // code
        // checksum (not validated)
        buf[2] = 0;
        buf[3] = 0;
        // identifier = 0x1234
        buf[4] = 0x12;
        buf[5] = 0x34;
        // sequence = 7
        buf[6] = 0x00;
        buf[7] = 0x07;

        let result = parse_icmp_response(&buf, 0x1234, 7, true);
        assert_eq!(result, Some(IcmpResponseType::EchoReply));
    }

    #[test]
    fn parse_icmpv6_response_identifies_time_exceeded() {
        // ICMPv6 time-exceeded: type 3, code 0
        // Payload: inner IPv6 header (40 bytes) + inner ICMPv6 echo request (8 bytes)
        let mut buf = vec![0u8; 8 + 40 + 8]; // 56 bytes

        // ICMPv6 header
        buf[0] = 3;  // type: time exceeded
        buf[1] = 0;  // code: hop limit exceeded

        // Inner IPv6 header starts at offset 8
        // Version (4 bits) + traffic class (8 bits) + flow label (20 bits)
        buf[8] = 0x60; // version 6
        // Next header at offset 6 within IPv6 header = offset 14 in buf
        buf[8 + 6] = 58; // ICMPv6 next-header

        // Inner IPv6 source address at offset 8 within IPv6 = offset 16 in buf
        // (router's address -- not what we check)

        // Inner IPv6 destination address at offset 24 within IPv6 = offset 32 in buf
        // (our target)

        // Inner ICMPv6 echo request starts at offset 8 + 40 = 48
        buf[48] = 128; // type: echo request
        buf[49] = 0;   // code
        // checksum
        buf[50] = 0;
        buf[51] = 0;
        // identifier = 0xBEEF
        buf[52] = 0xBE;
        buf[53] = 0xEF;
        // sequence = 42
        buf[54] = 0x00;
        buf[55] = 42;

        // The source IP for ICMPv6 time-exceeded comes from recvfrom(),
        // not from the packet buffer. We pass it separately via the socket addr.
        // For parsing, we extract the router IP from the recv address, but for
        // the unit test we just check the packet is recognized.
        let result = parse_icmp_response(&buf, 0xBEEF, 42, true);
        assert!(result.is_some(), "should recognize ICMPv6 time-exceeded");
        match result.unwrap() {
            IcmpResponseType::TimeExceeded(_) => {} // good
            other => panic!("expected TimeExceeded, got {:?}", other),
        }
    }

    #[test]
    fn parse_icmpv6_response_rejects_wrong_identifier() {
        let mut buf = vec![0u8; 8];
        buf[0] = 129; // echo reply
        buf[4] = 0xFF;
        buf[5] = 0xFF;
        buf[6] = 0x00;
        buf[7] = 0x01;

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, true), None);
    }

    #[test]
    fn parse_icmpv6_response_rejects_unknown_type() {
        let mut buf = vec![0u8; 8];
        buf[0] = 1; // destination unreachable (not handled for ICMP probe)
        buf[4] = 0x12;
        buf[5] = 0x34;
        buf[6] = 0x00;
        buf[7] = 0x01;

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, true), None);
    }

    #[test]
    fn parse_icmpv6_response_rejects_truncated_buffer() {
        // Too short for ICMPv6 header
        assert_eq!(parse_icmp_response(&[0u8; 4], 0x1234, 1, true), None);
    }

    #[test]
    fn parse_icmpv6_time_exceeded_rejects_truncated_inner() {
        // ICMPv6 time-exceeded header but not enough for inner IPv6 + ICMPv6
        let mut buf = vec![0u8; 20]; // only 20 bytes, need 56
        buf[0] = 3; // time exceeded
        buf[1] = 0;

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, true), None);
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

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, false), None);

        // Wrong sequence
        buf[24] = 0x12;
        buf[25] = 0x34;
        buf[26] = 0x00;
        buf[27] = 0x99; // wrong seq

        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, false), None);

        // Unknown ICMP type (e.g. type 3 = destination unreachable)
        buf[20] = 3;
        assert_eq!(parse_icmp_response(&buf, 0x1234, 1, false), None);

        // Buffer too short
        assert_eq!(parse_icmp_response(&[0u8; 10], 0x1234, 1, false), None);
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
