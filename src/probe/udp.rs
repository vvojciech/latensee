use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::probe::socket;
use crate::trace::state::ProbeResult;

/// Classification of an ICMP response to a UDP probe.
#[derive(Debug, PartialEq)]
pub enum UdpResponseType {
    /// Intermediate router returned ICMP time-exceeded (type 11).
    TimeExceeded(IpAddr),
    /// Final destination returned ICMP destination-unreachable/port-unreachable (type 3, code 3).
    PortUnreachable,
}

/// Parse a raw ICMP response buffer looking for responses to our UDP probe.
///
/// The buffer starts at the outer IP header (as received from a raw ICMP socket).
/// We check the inner IP+UDP headers to verify the response is for a packet
/// destined to `target`.
pub fn parse_icmp_for_udp(buf: &[u8], target: IpAddr) -> Option<UdpResponseType> {
    // Minimum: outer IP (20) + ICMP header (8) + inner IP (20) + inner UDP (8) = 56
    if buf.len() < 56 {
        return None;
    }

    let outer_ihl = ((buf[0] & 0x0F) as usize) * 4;
    if buf.len() < outer_ihl + 8 {
        return None;
    }

    let icmp_buf = &buf[outer_ihl..];
    let icmp_type = icmp_buf[0];
    let icmp_code = icmp_buf[1];

    // Inner IP header starts after ICMP header (8 bytes)
    let inner_ip_offset = outer_ihl + 8;
    if buf.len() < inner_ip_offset + 20 + 8 {
        return None;
    }

    let inner_ip = &buf[inner_ip_offset..];
    let inner_ihl = ((inner_ip[0] & 0x0F) as usize) * 4;
    if buf.len() < inner_ip_offset + inner_ihl + 8 {
        return None;
    }

    // Check inner protocol is UDP (17)
    let inner_protocol = inner_ip[9];
    if inner_protocol != 17 {
        return None;
    }

    // Check inner destination IP matches our target
    let inner_dest = IpAddr::V4(std::net::Ipv4Addr::new(
        inner_ip[16],
        inner_ip[17],
        inner_ip[18],
        inner_ip[19],
    ));
    if inner_dest != target {
        return None;
    }

    match icmp_type {
        // Time Exceeded (type 11)
        11 => {
            let src_ip = IpAddr::V4(std::net::Ipv4Addr::new(
                buf[12], buf[13], buf[14], buf[15],
            ));
            Some(UdpResponseType::TimeExceeded(src_ip))
        }
        // Destination Unreachable (type 3), code 3 = port unreachable
        3 if icmp_code == 3 => Some(UdpResponseType::PortUnreachable),
        _ => None,
    }
}

/// Send a single UDP probe and listen for an ICMP response.
///
/// Creates a UDP socket for sending and a raw ICMP socket for receiving
/// time-exceeded or port-unreachable responses.
pub async fn send_udp_probe(
    target: IpAddr,
    ttl: u8,
    seq: u16,
    timeout: Duration,
    base_port: u16,
) -> ProbeResult {
    let timestamp = Instant::now();

    let result = tokio::task::spawn_blocking(move || {
        let ipv6 = target.is_ipv6();

        // UDP send socket (no root needed)
        let send_sock = socket::create_udp_socket(ipv6)?;
        socket::set_ttl(&send_sock, ttl, ipv6)?;

        // Raw ICMP receive socket (needs root)
        let recv_sock = socket::create_icmp_socket(ipv6)?;
        socket::set_timeout(&recv_sock, timeout)?;

        let dest_port = base_port.wrapping_add(seq);
        let dest: socket2::SockAddr =
            std::net::SocketAddr::new(target, dest_port).into();

        let send_time = Instant::now();
        send_sock.send_to(&[], &dest)?;

        let mut recv_buf = [std::mem::MaybeUninit::<u8>::uninit(); 1500];
        loop {
            let elapsed = send_time.elapsed();
            if elapsed >= timeout {
                return Ok::<Option<Duration>, std::io::Error>(None);
            }

            match recv_sock.recv_from(&mut recv_buf) {
                Ok((n, _addr)) => {
                    let rtt = send_time.elapsed();
                    let received: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            recv_buf.as_ptr() as *const u8,
                            n,
                        )
                    };
                    if parse_icmp_for_udp(received, target).is_some() {
                        return Ok(Some(rtt));
                    }
                    // Not our packet, keep waiting
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return Ok(None);
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

    /// Build a fake ICMP-over-IP buffer for testing parse_icmp_for_udp.
    ///
    /// Outer IP: src_ip as source, minimal 20-byte header.
    /// ICMP: given type and code.
    /// Inner IP: protocol 17 (UDP), destination = inner_dest_ip, 20-byte header.
    /// Inner UDP: 8 bytes (src_port, dst_port, length, checksum).
    fn build_icmp_response(
        src_ip: Ipv4Addr,
        icmp_type: u8,
        icmp_code: u8,
        inner_dest_ip: Ipv4Addr,
        inner_protocol: u8,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 56];

        // Outer IP header (20 bytes)
        buf[0] = 0x45; // version 4, IHL 5
        buf[12] = src_ip.octets()[0];
        buf[13] = src_ip.octets()[1];
        buf[14] = src_ip.octets()[2];
        buf[15] = src_ip.octets()[3];

        // ICMP header (at offset 20)
        buf[20] = icmp_type;
        buf[21] = icmp_code;

        // Inner IP header (at offset 28)
        buf[28] = 0x45; // version 4, IHL 5
        buf[37] = inner_protocol; // protocol field at offset 9 within inner IP
        // Inner destination IP at offset 16 within inner IP
        buf[44] = inner_dest_ip.octets()[0];
        buf[45] = inner_dest_ip.octets()[1];
        buf[46] = inner_dest_ip.octets()[2];
        buf[47] = inner_dest_ip.octets()[3];

        // Inner UDP header (at offset 48): 8 bytes, left as zeros

        buf
    }

    #[test]
    fn parse_identifies_time_exceeded() {
        let target = Ipv4Addr::new(8, 8, 8, 8);
        let router = Ipv4Addr::new(10, 0, 0, 1);

        let buf = build_icmp_response(router, 11, 0, target, 17);

        let result = parse_icmp_for_udp(&buf, IpAddr::V4(target));
        assert_eq!(
            result,
            Some(UdpResponseType::TimeExceeded(IpAddr::V4(router)))
        );
    }

    #[test]
    fn parse_identifies_port_unreachable() {
        let target = Ipv4Addr::new(8, 8, 8, 8);

        let buf = build_icmp_response(target, 3, 3, target, 17);

        let result = parse_icmp_for_udp(&buf, IpAddr::V4(target));
        assert_eq!(result, Some(UdpResponseType::PortUnreachable));
    }

    #[test]
    fn parse_returns_none_for_unrelated_packets() {
        let target = Ipv4Addr::new(8, 8, 8, 8);
        let other = Ipv4Addr::new(1, 1, 1, 1);

        // Wrong inner destination (not our target)
        let buf = build_icmp_response(Ipv4Addr::new(10, 0, 0, 1), 11, 0, other, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target)), None);

        // Destination unreachable but wrong code (not port unreachable)
        let buf = build_icmp_response(target, 3, 1, target, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target)), None);

        // Inner protocol is not UDP (e.g. ICMP = 1)
        let buf = build_icmp_response(Ipv4Addr::new(10, 0, 0, 1), 11, 0, target, 1);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target)), None);

        // Unhandled ICMP type (e.g. echo reply = 0)
        let buf = build_icmp_response(target, 0, 0, target, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target)), None);
    }

    #[test]
    fn parse_returns_none_for_truncated_buffer() {
        let target = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        // Way too short
        assert_eq!(parse_icmp_for_udp(&[0u8; 10], target), None);

        // Just under minimum (55 bytes, need 56)
        assert_eq!(parse_icmp_for_udp(&[0u8; 55], target), None);

        // Exactly minimum but with garbage IP header (IHL = 0)
        let mut buf = vec![0u8; 56];
        buf[0] = 0x40; // version 4, IHL 0 (invalid)
        assert_eq!(parse_icmp_for_udp(&buf, target), None);
    }

    #[tokio::test]
    #[ignore = "requires root: raw ICMP socket needs CAP_NET_RAW or sudo"]
    async fn send_udp_probe_to_localhost() {
        let result = send_udp_probe(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            64,
            1,
            Duration::from_secs(2),
            33434,
        )
        .await;

        assert_eq!(result.seq, 1);
        // Localhost UDP probe should get a port-unreachable back quickly
        if let Some(rtt) = result.rtt {
            assert!(
                rtt < Duration::from_millis(100),
                "localhost RTT should be under 100ms, got {:?}",
                rtt
            );
        }
    }
}
