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
/// **IPv4:** buffer starts at outer IP header (raw ICMP sockets include it).
/// **IPv6:** buffer starts at ICMPv6 header (kernel strips IPv6 header).
///
/// We check the inner IP+UDP headers to verify the response is for a packet
/// destined to `target`.
pub fn parse_icmp_for_udp(buf: &[u8], target: IpAddr, ipv6: bool) -> Option<UdpResponseType> {
    if ipv6 {
        parse_icmpv6_for_udp(buf, target)
    } else {
        parse_icmpv4_for_udp(buf, target)
    }
}

/// Parse an ICMPv4 response for a UDP probe (buffer includes IPv4 header).
fn parse_icmpv4_for_udp(buf: &[u8], target: IpAddr) -> Option<UdpResponseType> {
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

/// Parse an ICMPv6 response for a UDP probe (buffer starts at ICMPv6 header).
///
/// ICMPv6 type differences from ICMPv4:
/// - Time Exceeded: type 3 (vs 11)
/// - Destination Unreachable: type 1 (vs 3), port unreachable code 4 (vs 3)
///
/// Inner packet is IPv6 (40-byte fixed header, next-header at offset 6, no IHL).
fn parse_icmpv6_for_udp(buf: &[u8], target: IpAddr) -> Option<UdpResponseType> {
    // Minimum: ICMPv6 header (8) + inner IPv6 header (40) + inner UDP (8) = 56
    if buf.len() < 56 {
        return None;
    }

    let icmp_type = buf[0];
    let icmp_code = buf[1];

    // Inner IPv6 header starts at offset 8
    let inner_ipv6 = &buf[8..];

    // Check next-header field (offset 6 in IPv6 header) is UDP (17)
    let next_header = inner_ipv6[6];
    if next_header != 17 {
        return None;
    }

    // Inner destination address at offset 24 in IPv6 header (16 bytes)
    let mut dest_bytes = [0u8; 16];
    dest_bytes.copy_from_slice(&inner_ipv6[24..40]);
    let inner_dest = IpAddr::V6(std::net::Ipv6Addr::from(dest_bytes));
    if inner_dest != target {
        return None;
    }

    match icmp_type {
        // ICMPv6 Time Exceeded (type 3)
        3 => {
            // Router IP comes from recvfrom(), use unspecified as placeholder
            let src_ip = IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
            Some(UdpResponseType::TimeExceeded(src_ip))
        }
        // ICMPv6 Destination Unreachable (type 1), code 4 = port unreachable
        1 if icmp_code == 4 => Some(UdpResponseType::PortUnreachable),
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
                    if parse_icmp_for_udp(received, target, ipv6).is_some() {
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
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    /// Build a fake ICMPv4-over-IPv4 buffer for testing parse_icmp_for_udp.
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

    /// Build a fake ICMPv6 buffer for testing parse_icmpv6_for_udp.
    ///
    /// ICMPv6 raw sockets don't include IPv6 header.
    /// Layout: ICMPv6 header (8) + inner IPv6 header (40) + inner UDP (8) = 56 bytes.
    fn build_icmpv6_response(
        icmp_type: u8,
        icmp_code: u8,
        inner_dest_ip: Ipv6Addr,
        inner_next_header: u8,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 56]; // 8 + 40 + 8

        // ICMPv6 header
        buf[0] = icmp_type;
        buf[1] = icmp_code;

        // Inner IPv6 header starts at offset 8
        buf[8] = 0x60; // version 6
        // Next header at offset 6 within IPv6 header
        buf[8 + 6] = inner_next_header;
        // Inner destination address at offset 24 within IPv6 header = offset 32
        let dest_octets = inner_dest_ip.octets();
        buf[32..48].copy_from_slice(&dest_octets);

        // Inner UDP header at offset 48: 8 bytes, left as zeros

        buf
    }

    #[test]
    fn parse_identifies_time_exceeded() {
        let target = Ipv4Addr::new(8, 8, 8, 8);
        let router = Ipv4Addr::new(10, 0, 0, 1);

        let buf = build_icmp_response(router, 11, 0, target, 17);

        let result = parse_icmp_for_udp(&buf, IpAddr::V4(target), false);
        assert_eq!(
            result,
            Some(UdpResponseType::TimeExceeded(IpAddr::V4(router)))
        );
    }

    #[test]
    fn parse_identifies_port_unreachable() {
        let target = Ipv4Addr::new(8, 8, 8, 8);

        let buf = build_icmp_response(target, 3, 3, target, 17);

        let result = parse_icmp_for_udp(&buf, IpAddr::V4(target), false);
        assert_eq!(result, Some(UdpResponseType::PortUnreachable));
    }

    #[test]
    fn parse_returns_none_for_unrelated_packets() {
        let target = Ipv4Addr::new(8, 8, 8, 8);
        let other = Ipv4Addr::new(1, 1, 1, 1);

        // Wrong inner destination (not our target)
        let buf = build_icmp_response(Ipv4Addr::new(10, 0, 0, 1), 11, 0, other, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target), false), None);

        // Destination unreachable but wrong code (not port unreachable)
        let buf = build_icmp_response(target, 3, 1, target, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target), false), None);

        // Inner protocol is not UDP (e.g. ICMP = 1)
        let buf = build_icmp_response(Ipv4Addr::new(10, 0, 0, 1), 11, 0, target, 1);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target), false), None);

        // Unhandled ICMP type (e.g. echo reply = 0)
        let buf = build_icmp_response(target, 0, 0, target, 17);
        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V4(target), false), None);
    }

    #[test]
    fn parse_returns_none_for_truncated_buffer() {
        let target = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        // Way too short
        assert_eq!(parse_icmp_for_udp(&[0u8; 10], target, false), None);

        // Just under minimum (55 bytes, need 56)
        assert_eq!(parse_icmp_for_udp(&[0u8; 55], target, false), None);

        // Exactly minimum but with garbage IP header (IHL = 0)
        let mut buf = vec![0u8; 56];
        buf[0] = 0x40; // version 4, IHL 0 (invalid)
        assert_eq!(parse_icmp_for_udp(&buf, target, false), None);
    }

    // --- IPv6 UDP parse tests ---

    #[test]
    fn parse_v6_identifies_time_exceeded() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

        let buf = build_icmpv6_response(3, 0, target, 17); // type 3 = time exceeded, next-header 17 = UDP

        let result = parse_icmp_for_udp(&buf, IpAddr::V6(target), true);
        assert!(result.is_some(), "should parse ICMPv6 time-exceeded for UDP");
        match result.unwrap() {
            UdpResponseType::TimeExceeded(_) => {} // router IP is placeholder
            other => panic!("expected TimeExceeded, got {:?}", other),
        }
    }

    #[test]
    fn parse_v6_identifies_port_unreachable() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

        // ICMPv6 destination unreachable (type 1), code 4 = port unreachable
        let buf = build_icmpv6_response(1, 4, target, 17);

        let result = parse_icmp_for_udp(&buf, IpAddr::V6(target), true);
        assert_eq!(result, Some(UdpResponseType::PortUnreachable));
    }

    #[test]
    fn parse_v6_rejects_wrong_inner_destination() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let other = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2);

        let buf = build_icmpv6_response(3, 0, other, 17);

        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V6(target), true), None);
    }

    #[test]
    fn parse_v6_rejects_non_udp_inner_protocol() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

        // Inner next-header = 6 (TCP), not UDP
        let buf = build_icmpv6_response(3, 0, target, 6);

        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V6(target), true), None);
    }

    #[test]
    fn parse_v6_rejects_wrong_unreachable_code() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

        // Destination unreachable type 1 but code 0 (no route), not port unreachable
        let buf = build_icmpv6_response(1, 0, target, 17);

        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V6(target), true), None);
    }

    #[test]
    fn parse_v6_rejects_truncated_buffer() {
        let target = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));

        assert_eq!(parse_icmp_for_udp(&[0u8; 10], target, true), None);
        assert_eq!(parse_icmp_for_udp(&[0u8; 55], target, true), None);
    }

    #[test]
    fn parse_v6_rejects_unhandled_icmpv6_type() {
        let target = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);

        // Type 129 = echo reply, not relevant for UDP probe
        let buf = build_icmpv6_response(129, 0, target, 17);

        assert_eq!(parse_icmp_for_udp(&buf, IpAddr::V6(target), true), None);
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
