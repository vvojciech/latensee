use std::mem::MaybeUninit;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use pnet::packet::tcp::{ipv4_checksum, ipv6_checksum, MutableTcpPacket, TcpFlags, TcpPacket};

use crate::probe::socket;
use crate::trace::state::ProbeResult;

/// Classification of a TCP probe response.
#[derive(Debug, PartialEq)]
pub enum TcpResponseType {
    /// ICMP time-exceeded from an intermediate router.
    TimeExceeded(IpAddr),
    /// TCP SYN-ACK from the target (port open).
    SynAck,
    /// TCP RST from the target (port closed, but host reachable).
    Reset,
}

const TCP_HEADER_LEN: usize = 20;

/// Build a TCP SYN packet (header only, no payload).
///
/// Returns raw bytes suitable for sending on a raw TCP socket.
/// Computes the pseudo-header checksum for IPv4 targets.
pub fn build_tcp_syn(src_port: u16, dst_port: u16, target: IpAddr, seq_num: u32) -> Vec<u8> {
    let mut buf = vec![0u8; TCP_HEADER_LEN];

    {
        let mut tcp = MutableTcpPacket::new(&mut buf).expect("buffer too small for TCP header");
        tcp.set_source(src_port);
        tcp.set_destination(dst_port);
        tcp.set_sequence(seq_num);
        tcp.set_acknowledgement(0);
        tcp.set_data_offset(5); // 20 bytes / 4 = 5 (no options)
        tcp.set_flags(TcpFlags::SYN);
        tcp.set_window(64240); // typical SYN window
        tcp.set_checksum(0);
        tcp.set_urgent_ptr(0);
    }

    // Compute checksum with pseudo-header
    match target {
        IpAddr::V4(dst) => {
            // Use 0.0.0.0 as source for checksum; the kernel fills the real source IP.
            // For raw sockets with IP_HDRINCL off, this is standard practice.
            let src = Ipv4Addr::UNSPECIFIED;
            let tcp_packet = TcpPacket::new(&buf).expect("failed to parse TCP for checksum");
            let cksum = ipv4_checksum(&tcp_packet, &src, &dst);
            buf[16] = (cksum >> 8) as u8;
            buf[17] = (cksum & 0xFF) as u8;
        }
        IpAddr::V6(dst) => {
            let src = Ipv6Addr::UNSPECIFIED;
            let tcp_packet = TcpPacket::new(&buf).expect("failed to parse TCP for checksum");
            let cksum = ipv6_checksum(&tcp_packet, &src, &dst);
            buf[16] = (cksum >> 8) as u8;
            buf[17] = (cksum & 0xFF) as u8;
        }
    }

    buf
}

/// Parse a received buffer for TCP probe responses.
///
/// Handles two response categories:
/// 1. ICMP time-exceeded containing our original TCP header (intermediate hop)
/// 2. TCP SYN-ACK or RST from the target (final destination)
///
/// **IPv4:** `buf` starts at the IP header (raw socket includes it).
/// **IPv6:** For ICMP socket: `buf` starts at ICMPv6 header (kernel strips IPv6).
///           For TCP socket: `buf` starts at the TCP header directly.
pub fn parse_tcp_response(
    buf: &[u8],
    src_port: u16,
    dst_port: u16,
    ipv6: bool,
) -> Option<TcpResponseType> {
    if ipv6 {
        parse_tcp_response_v6(buf, src_port, dst_port)
    } else {
        parse_tcp_response_v4(buf, src_port, dst_port)
    }
}

/// Parse an IPv4 response for a TCP probe.
fn parse_tcp_response_v4(
    buf: &[u8],
    src_port: u16,
    dst_port: u16,
) -> Option<TcpResponseType> {
    if buf.len() < 20 {
        return None;
    }

    let ip_version = (buf[0] >> 4) & 0x0F;
    if ip_version != 4 {
        return None;
    }

    let ip_header_len = ((buf[0] & 0x0F) as usize) * 4;
    let protocol = buf[9];

    match protocol {
        // ICMP (protocol 1): check for time-exceeded containing our TCP
        1 => parse_icmpv4_time_exceeded(buf, ip_header_len, src_port, dst_port),
        // TCP (protocol 6): check for SYN-ACK or RST matching our ports
        6 => parse_tcp_reply(buf, ip_header_len, src_port, dst_port),
        _ => None,
    }
}

/// Parse an IPv6 response for a TCP probe.
///
/// This handles two buffer formats:
/// - ICMPv6 socket: starts at ICMPv6 header (type 3 = time-exceeded)
/// - TCP socket: starts at TCP header directly (kernel strips IPv6 header)
fn parse_tcp_response_v6(
    buf: &[u8],
    src_port: u16,
    dst_port: u16,
) -> Option<TcpResponseType> {
    if buf.len() < 8 {
        return None;
    }

    let first_byte = buf[0];

    // ICMPv6 time-exceeded: type 3
    if first_byte == 3 {
        return parse_icmpv6_time_exceeded(buf, src_port, dst_port);
    }

    // Try parsing as a raw TCP header (from TCP raw socket).
    // TCP data offset is in bits 4-7 of byte 12, and must be >= 5.
    if buf.len() >= 14 {
        return parse_tcp_reply(buf, 0, src_port, dst_port);
    }

    None
}

/// Parse ICMPv4 time-exceeded that embeds our original TCP header.
fn parse_icmpv4_time_exceeded(
    buf: &[u8],
    ip_header_len: usize,
    src_port: u16,
    dst_port: u16,
) -> Option<TcpResponseType> {
    // Need: outer IP + ICMP header (8) + inner IP header (20) + inner TCP ports (4)
    if buf.len() < ip_header_len + 8 + 20 + 4 {
        return None;
    }

    let icmp = &buf[ip_header_len..];
    let icmp_type = icmp[0];

    // Type 11 = time exceeded
    if icmp_type != 11 {
        return None;
    }

    // Inner IP header starts after ICMP header (8 bytes)
    let inner_ip = &icmp[8..];
    let inner_protocol = inner_ip[9];
    if inner_protocol != 6 {
        return None; // Not TCP inside
    }

    let inner_ip_header_len = ((inner_ip[0] & 0x0F) as usize) * 4;
    if icmp.len() < 8 + inner_ip_header_len + 4 {
        return None;
    }

    let inner_tcp = &icmp[8 + inner_ip_header_len..];
    let orig_src_port = u16::from_be_bytes([inner_tcp[0], inner_tcp[1]]);
    let orig_dst_port = u16::from_be_bytes([inner_tcp[2], inner_tcp[3]]);

    if orig_src_port == src_port && orig_dst_port == dst_port {
        let router_ip = IpAddr::V4(Ipv4Addr::new(buf[12], buf[13], buf[14], buf[15]));
        Some(TcpResponseType::TimeExceeded(router_ip))
    } else {
        None
    }
}

/// Parse ICMPv6 time-exceeded that embeds our original TCP header.
///
/// Buffer starts at ICMPv6 header. Inner packet is IPv6 (40-byte header).
fn parse_icmpv6_time_exceeded(
    buf: &[u8],
    src_port: u16,
    dst_port: u16,
) -> Option<TcpResponseType> {
    // Need: ICMPv6 header (8) + inner IPv6 header (40) + inner TCP ports (4) = 52
    if buf.len() < 52 {
        return None;
    }

    let icmp_type = buf[0];
    if icmp_type != 3 {
        return None;
    }

    // Inner IPv6 header starts at offset 8
    let inner_ipv6 = &buf[8..];
    let next_header = inner_ipv6[6];
    if next_header != 6 {
        return None; // Not TCP inside
    }

    // Inner TCP starts at offset 8 + 40 = 48
    let inner_tcp = &buf[48..];
    let orig_src_port = u16::from_be_bytes([inner_tcp[0], inner_tcp[1]]);
    let orig_dst_port = u16::from_be_bytes([inner_tcp[2], inner_tcp[3]]);

    if orig_src_port == src_port && orig_dst_port == dst_port {
        // Router IP comes from recvfrom(), placeholder here
        let router_ip = IpAddr::V6(Ipv6Addr::UNSPECIFIED);
        Some(TcpResponseType::TimeExceeded(router_ip))
    } else {
        None
    }
}

/// Parse a TCP reply (SYN-ACK or RST) matching our ports.
///
/// `offset` is 0 for IPv6 (buffer starts at TCP header) or `ip_header_len` for IPv4.
fn parse_tcp_reply(
    buf: &[u8],
    offset: usize,
    src_port: u16,
    dst_port: u16,
) -> Option<TcpResponseType> {
    if buf.len() < offset + 14 {
        return None;
    }

    let tcp = &buf[offset..];

    // In the reply, ports are swapped: target replies from dst_port to our src_port
    let reply_src_port = u16::from_be_bytes([tcp[0], tcp[1]]);
    let reply_dst_port = u16::from_be_bytes([tcp[2], tcp[3]]);

    if reply_src_port != dst_port || reply_dst_port != src_port {
        return None;
    }

    // Flags are in byte 13 of TCP header
    let flags = tcp[13];
    let syn = flags & TcpFlags::SYN;
    let ack = flags & TcpFlags::ACK;
    let rst = flags & TcpFlags::RST;

    if syn != 0 && ack != 0 {
        Some(TcpResponseType::SynAck)
    } else if rst != 0 {
        Some(TcpResponseType::Reset)
    } else {
        None
    }
}

/// Send a single TCP SYN probe and wait for a response.
///
/// Creates a raw TCP socket for sending and a raw ICMP socket for receiving
/// time-exceeded messages from intermediate routers.
pub async fn send_tcp_probe(
    target: IpAddr,
    ttl: u8,
    seq: u16,
    timeout: Duration,
    port: u16,
) -> ProbeResult {
    let timestamp = Instant::now();
    let src_port = 30000 + seq;

    let result = tokio::task::spawn_blocking(move || {
        let ipv6 = target.is_ipv6();

        let tcp_sock = socket::create_tcp_socket(ipv6)?;
        socket::set_ttl(&tcp_sock, ttl, ipv6)?;
        socket::set_timeout(&tcp_sock, timeout)?;

        let icmp_sock = socket::create_icmp_socket(ipv6)?;
        socket::set_timeout(&icmp_sock, timeout)?;

        let packet = build_tcp_syn(src_port, port, target, seq as u32);
        let dest: socket2::SockAddr = std::net::SocketAddr::new(target, 0).into();

        let send_time = Instant::now();
        tcp_sock.send_to(&packet, &dest)?;

        let mut recv_buf = [MaybeUninit::<u8>::uninit(); 1500];

        loop {
            let elapsed = send_time.elapsed();
            if elapsed >= timeout {
                return Ok::<Option<Duration>, std::io::Error>(None);
            }

            // Try ICMP socket (time-exceeded from intermediate routers)
            match icmp_sock.recv_from(&mut recv_buf) {
                Ok((n, _addr)) => {
                    let rtt = send_time.elapsed();
                    let received: &[u8] =
                        unsafe { std::slice::from_raw_parts(recv_buf.as_ptr() as *const u8, n) };
                    if parse_tcp_response(received, src_port, port, ipv6).is_some() {
                        return Ok(Some(rtt));
                    }
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e),
            }

            // Try TCP socket (SYN-ACK or RST from target)
            match tcp_sock.recv_from(&mut recv_buf) {
                Ok((n, _addr)) => {
                    let rtt = send_time.elapsed();
                    let received: &[u8] =
                        unsafe { std::slice::from_raw_parts(recv_buf.as_ptr() as *const u8, n) };
                    if parse_tcp_response(received, src_port, port, ipv6).is_some() {
                        return Ok(Some(rtt));
                    }
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
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

    #[test]
    fn build_tcp_syn_produces_valid_header_with_syn_flag() {
        let packet = build_tcp_syn(30001, 80, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 42);

        assert_eq!(packet.len(), TCP_HEADER_LEN, "TCP header should be 20 bytes");

        let tcp = TcpPacket::new(&packet).expect("should parse as valid TCP packet");

        // SYN flag must be set
        assert_ne!(tcp.get_flags() & TcpFlags::SYN, 0, "SYN flag must be set");
        // ACK flag must NOT be set
        assert_eq!(
            tcp.get_flags() & TcpFlags::ACK,
            0,
            "ACK flag must not be set on SYN"
        );
        // Data offset = 5 (20 bytes, no options)
        assert_eq!(tcp.get_data_offset(), 5, "data offset should be 5");
    }

    #[test]
    fn build_tcp_syn_has_correct_ports_and_seq() {
        let packet = build_tcp_syn(30042, 443, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 99);

        let tcp = TcpPacket::new(&packet).expect("should parse as valid TCP packet");
        assert_eq!(tcp.get_source(), 30042, "source port mismatch");
        assert_eq!(tcp.get_destination(), 443, "destination port mismatch");
        assert_eq!(tcp.get_sequence(), 99, "sequence number mismatch");
    }

    #[test]
    fn build_tcp_syn_checksum_is_nonzero_v4() {
        let packet = build_tcp_syn(30001, 80, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 1);
        let tcp = TcpPacket::new(&packet).expect("should parse as valid TCP packet");
        assert_ne!(
            tcp.get_checksum(),
            0,
            "IPv4 checksum should be computed (non-zero)"
        );
    }

    #[test]
    fn build_tcp_syn_ipv6_uses_ipv6_checksum() {
        let target = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        let packet = build_tcp_syn(30001, 80, target, 1);

        let tcp = TcpPacket::new(&packet).expect("should parse as valid TCP packet");
        assert_ne!(
            tcp.get_checksum(),
            0,
            "IPv6 checksum should be computed (non-zero)"
        );

        // Verify it matches pnet's ipv6_checksum computation
        let expected = ipv6_checksum(
            &tcp,
            &Ipv6Addr::UNSPECIFIED,
            &Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
        );
        assert_eq!(tcp.get_checksum(), expected, "checksum should match pnet's ipv6_checksum");
    }

    #[test]
    fn build_tcp_syn_ipv6_has_syn_flag_and_correct_ports() {
        let target = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        let packet = build_tcp_syn(30042, 443, target, 99);

        let tcp = TcpPacket::new(&packet).expect("should parse as valid TCP packet");
        assert_ne!(tcp.get_flags() & TcpFlags::SYN, 0, "SYN flag must be set");
        assert_eq!(tcp.get_source(), 30042);
        assert_eq!(tcp.get_destination(), 443);
        assert_eq!(tcp.get_sequence(), 99);
    }

    #[test]
    fn parse_tcp_response_identifies_time_exceeded_with_matching_tcp() {
        // Outer IP header (20) + ICMP time-exceeded (8) + inner IP header (20) + inner TCP (8)
        let mut buf = vec![0u8; 56];

        // Outer IPv4 header: version 4, IHL 5
        buf[0] = 0x45;
        buf[9] = 1; // protocol = ICMP
        // Router source IP: 10.0.0.1
        buf[12] = 10;
        buf[13] = 0;
        buf[14] = 0;
        buf[15] = 1;

        // ICMP time-exceeded: type 11, code 0
        buf[20] = 11;
        buf[21] = 0;

        // Inner IP header at offset 28
        buf[28] = 0x45; // version 4, IHL 5
        buf[37] = 6; // protocol = TCP

        // Inner TCP at offset 48: src_port=30001, dst_port=80
        buf[48] = (30001u16 >> 8) as u8;
        buf[49] = (30001u16 & 0xFF) as u8;
        buf[50] = 0;
        buf[51] = 80;

        let result = parse_tcp_response(&buf, 30001, 80, false);
        assert_eq!(
            result,
            Some(TcpResponseType::TimeExceeded(IpAddr::V4(Ipv4Addr::new(
                10, 0, 0, 1
            ))))
        );
    }

    #[test]
    fn parse_tcp_response_identifies_syn_ack() {
        // IP header (20) + TCP header (20)
        let mut buf = vec![0u8; 40];

        // IPv4 header
        buf[0] = 0x45;
        buf[9] = 6; // protocol = TCP

        // TCP: src=80 (target replies from its port), dst=30001 (our src port)
        buf[20] = 0;
        buf[21] = 80;
        buf[22] = (30001u16 >> 8) as u8;
        buf[23] = (30001u16 & 0xFF) as u8;

        // Flags byte (offset 13 within TCP = offset 33 in buffer)
        buf[33] = TcpFlags::SYN | TcpFlags::ACK;

        let result = parse_tcp_response(&buf, 30001, 80, false);
        assert_eq!(result, Some(TcpResponseType::SynAck));
    }

    #[test]
    fn parse_tcp_response_identifies_rst() {
        let mut buf = vec![0u8; 40];
        buf[0] = 0x45;
        buf[9] = 6; // TCP

        buf[20] = 0;
        buf[21] = 80;
        buf[22] = (30001u16 >> 8) as u8;
        buf[23] = (30001u16 & 0xFF) as u8;

        buf[33] = TcpFlags::RST | TcpFlags::ACK;

        let result = parse_tcp_response(&buf, 30001, 80, false);
        assert_eq!(result, Some(TcpResponseType::Reset));
    }

    #[test]
    fn parse_tcp_response_returns_none_for_unrelated_packets() {
        // Wrong ports in time-exceeded
        let mut buf = vec![0u8; 56];
        buf[0] = 0x45;
        buf[9] = 1; // ICMP
        buf[12] = 10;
        buf[20] = 11; // time-exceeded
        buf[28] = 0x45;
        buf[37] = 6; // inner TCP
        buf[48] = 0xFF; // wrong src port
        buf[49] = 0xFF;
        buf[50] = 0;
        buf[51] = 80;

        assert_eq!(parse_tcp_response(&buf, 30001, 80, false), None);

        // Wrong ports in TCP reply
        let mut buf2 = vec![0u8; 40];
        buf2[0] = 0x45;
        buf2[9] = 6;
        buf2[20] = 0;
        buf2[21] = 0x01; // port 443 high byte
        buf2[22] = (30001u16 >> 8) as u8;
        buf2[23] = (30001u16 & 0xFF) as u8;
        buf2[33] = TcpFlags::SYN | TcpFlags::ACK;

        // reply_src_port would be 1, not 80
        assert_eq!(parse_tcp_response(&buf2, 30001, 80, false), None);

        // Buffer too short
        assert_eq!(parse_tcp_response(&[0u8; 10], 30001, 80, false), None);

        // Unknown protocol (UDP = 17)
        let mut buf3 = vec![0u8; 40];
        buf3[0] = 0x45;
        buf3[9] = 17;
        assert_eq!(parse_tcp_response(&buf3, 30001, 80, false), None);
    }

    #[test]
    fn parse_tcp_response_returns_none_for_non_tcp_inner_protocol() {
        // ICMP time-exceeded but inner packet is UDP
        let mut buf = vec![0u8; 56];
        buf[0] = 0x45;
        buf[9] = 1; // ICMP
        buf[20] = 11; // time-exceeded
        buf[28] = 0x45;
        buf[37] = 17; // inner protocol = UDP
        buf[48] = (30001u16 >> 8) as u8;
        buf[49] = (30001u16 & 0xFF) as u8;
        buf[50] = 0;
        buf[51] = 80;

        assert_eq!(parse_tcp_response(&buf, 30001, 80, false), None);
    }

    // --- IPv6 TCP parse tests ---

    #[test]
    fn parse_tcp_response_v6_identifies_icmpv6_time_exceeded() {
        // ICMPv6 header (8) + inner IPv6 header (40) + inner TCP ports (4) = 52
        let mut buf = vec![0u8; 52];

        // ICMPv6 time-exceeded: type 3, code 0
        buf[0] = 3;
        buf[1] = 0;

        // Inner IPv6 header at offset 8
        buf[8] = 0x60; // version 6
        buf[8 + 6] = 6; // next-header = TCP

        // Inner TCP at offset 48: src_port=30001, dst_port=80
        buf[48] = (30001u16 >> 8) as u8;
        buf[49] = (30001u16 & 0xFF) as u8;
        buf[50] = 0;
        buf[51] = 80;

        let result = parse_tcp_response(&buf, 30001, 80, true);
        assert!(result.is_some(), "should parse ICMPv6 time-exceeded for TCP");
        match result.unwrap() {
            TcpResponseType::TimeExceeded(_) => {}
            other => panic!("expected TimeExceeded, got {:?}", other),
        }
    }

    #[test]
    fn parse_tcp_response_v6_identifies_syn_ack_from_tcp_socket() {
        // IPv6 TCP socket delivers raw TCP header (no IPv6 header)
        let mut buf = vec![0u8; 20];

        // TCP: src=80, dst=30001
        buf[0] = 0;
        buf[1] = 80;
        buf[2] = (30001u16 >> 8) as u8;
        buf[3] = (30001u16 & 0xFF) as u8;

        // Flags at byte 13
        buf[13] = TcpFlags::SYN | TcpFlags::ACK;

        let result = parse_tcp_response(&buf, 30001, 80, true);
        assert_eq!(result, Some(TcpResponseType::SynAck));
    }

    #[test]
    fn parse_tcp_response_v6_identifies_rst_from_tcp_socket() {
        let mut buf = vec![0u8; 20];
        buf[0] = 0;
        buf[1] = 80;
        buf[2] = (30001u16 >> 8) as u8;
        buf[3] = (30001u16 & 0xFF) as u8;
        buf[13] = TcpFlags::RST | TcpFlags::ACK;

        let result = parse_tcp_response(&buf, 30001, 80, true);
        assert_eq!(result, Some(TcpResponseType::Reset));
    }

    #[test]
    fn parse_tcp_response_v6_rejects_wrong_ports_in_time_exceeded() {
        let mut buf = vec![0u8; 52];
        buf[0] = 3; // ICMPv6 time-exceeded
        buf[8] = 0x60;
        buf[8 + 6] = 6; // TCP
        buf[48] = 0xFF; // wrong src port
        buf[49] = 0xFF;
        buf[50] = 0;
        buf[51] = 80;

        assert_eq!(parse_tcp_response(&buf, 30001, 80, true), None);
    }

    #[test]
    fn parse_tcp_response_v6_rejects_non_tcp_inner_protocol() {
        let mut buf = vec![0u8; 52];
        buf[0] = 3; // ICMPv6 time-exceeded
        buf[8] = 0x60;
        buf[8 + 6] = 17; // UDP, not TCP

        assert_eq!(parse_tcp_response(&buf, 30001, 80, true), None);
    }

    #[test]
    fn parse_tcp_response_v6_rejects_truncated_buffer() {
        assert_eq!(parse_tcp_response(&[0u8; 4], 30001, 80, true), None);
    }

    #[tokio::test]
    #[ignore = "requires root: raw TCP and ICMP sockets need CAP_NET_RAW or sudo"]
    async fn send_tcp_probe_integration() {
        let result = send_tcp_probe(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            64,
            1,
            Duration::from_secs(2),
            80,
        )
        .await;

        assert_eq!(result.seq, 1);
        // On localhost with no service on port 80, we expect RST or timeout.
        // Either is valid; just verify it completes without panic.
    }
}
