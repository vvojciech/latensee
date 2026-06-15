use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::time::Duration;

/// Create a raw ICMP socket. Uses ICMPv6 protocol when `ipv6` is true.
pub fn create_icmp_socket(ipv6: bool) -> io::Result<Socket> {
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };
    let protocol = if ipv6 {
        Protocol::ICMPV6
    } else {
        Protocol::ICMPV4
    };
    Socket::new(domain, Type::RAW, Some(protocol))
}

/// Set the TTL (or hop limit for IPv6) on a socket.
pub fn set_ttl(socket: &Socket, ttl: u8, ipv6: bool) -> io::Result<()> {
    if ipv6 {
        socket.set_unicast_hops_v6(ttl as u32)
    } else {
        socket.set_ttl(ttl as u32)
    }
}

/// Set send and receive timeouts on a socket.
pub fn set_timeout(socket: &Socket, timeout: Duration) -> io::Result<()> {
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    Ok(())
}
