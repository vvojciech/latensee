// Raw socket abstraction for ICMP/UDP/TCP probes

use socket2::{Domain, Protocol, Socket, Type};
use std::time::Duration;
use thiserror::Error;

/// Error returned when the process lacks raw socket privileges.
#[derive(Debug, Error)]
#[error("Insufficient privileges to create raw sockets. {hint}")]
pub struct PrivilegeError {
    hint: &'static str,
}

impl PrivilegeError {
    fn new() -> Self {
        Self {
            hint: Self::platform_hint(),
        }
    }

    #[cfg(target_os = "macos")]
    fn platform_hint() -> &'static str {
        "Run with sudo: sudo latensee <target>"
    }

    #[cfg(target_os = "linux")]
    fn platform_hint() -> &'static str {
        "Run with sudo or set capabilities: sudo setcap cap_net_raw+ep $(which latensee)"
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn platform_hint() -> &'static str {
        "Run with elevated privileges"
    }
}

/// Attempt to create a raw ICMP socket to verify we have privileges.
/// The socket is dropped immediately - this is a check only.
pub fn check_privileges() -> Result<(), PrivilegeError> {
    Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4))
        .map(drop)
        .map_err(|_| PrivilegeError::new())
}

/// Create a raw ICMP socket for sending probes and receiving replies.
/// IPv4: AF_INET + SOCK_RAW + IPPROTO_ICMP
/// IPv6: AF_INET6 + SOCK_RAW + IPPROTO_ICMPV6
pub fn create_icmp_socket(ipv6: bool) -> Result<Socket, std::io::Error> {
    let (domain, protocol) = if ipv6 {
        (Domain::IPV6, Protocol::ICMPV6)
    } else {
        (Domain::IPV4, Protocol::ICMPV4)
    };
    let socket = Socket::new(domain, Type::RAW, Some(protocol))?;
    Ok(socket)
}

/// Create a UDP socket for traceroute probes.
/// Uses SOCK_DGRAM (no raw socket privilege needed).
pub fn create_udp_socket(ipv6: bool) -> Result<Socket, std::io::Error> {
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };
    Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
}

/// Create a raw TCP socket for TCP-based probes.
/// IPv4: AF_INET + SOCK_RAW + IPPROTO_TCP
/// IPv6: AF_INET6 + SOCK_RAW + IPPROTO_TCP
pub fn create_tcp_socket(ipv6: bool) -> Result<Socket, std::io::Error> {
    let domain = if ipv6 { Domain::IPV6 } else { Domain::IPV4 };
    Socket::new(domain, Type::RAW, Some(Protocol::TCP))
}

/// Set the TTL (Time To Live) on a socket.
/// IPv4: IP_TTL
/// IPv6: IPV6_UNICAST_HOPS
pub fn set_ttl(socket: &Socket, ttl: u8, ipv6: bool) -> Result<(), std::io::Error> {
    if ipv6 {
        socket.set_unicast_hops_v6(ttl.into())
    } else {
        socket.set_ttl(ttl.into())
    }
}

/// Set the receive timeout (SO_RCVTIMEO) on a socket.
pub fn set_timeout(socket: &Socket, timeout: Duration) -> Result<(), std::io::Error> {
    socket.set_read_timeout(Some(timeout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privilege_error_contains_platform_hint() {
        let err = PrivilegeError::new();
        let msg = err.to_string();

        assert!(
            msg.contains("Insufficient privileges"),
            "should mention insufficient privileges: {msg}"
        );

        #[cfg(target_os = "macos")]
        assert!(msg.contains("sudo latensee"), "macOS hint missing: {msg}");

        #[cfg(target_os = "linux")]
        assert!(msg.contains("setcap"), "Linux hint missing: {msg}");
    }

    #[test]
    fn create_udp_socket_ipv4_succeeds() {
        let socket = create_udp_socket(false);
        assert!(socket.is_ok(), "UDP IPv4 socket should not need root");
    }

    #[test]
    fn create_udp_socket_ipv6_succeeds() {
        let socket = create_udp_socket(true);
        assert!(socket.is_ok(), "UDP IPv6 socket should not need root");
    }

    #[test]
    fn set_ttl_on_udp_socket_succeeds() {
        let socket = create_udp_socket(false).expect("UDP socket creation failed");
        let result = set_ttl(&socket, 5, false);
        assert!(result.is_ok(), "set_ttl should succeed on UDP socket");
    }

    #[test]
    fn set_ttl_value_is_applied() {
        let socket = create_udp_socket(false).expect("UDP socket creation failed");
        set_ttl(&socket, 42, false).expect("set_ttl failed");
        let actual = socket.ttl().expect("failed to read TTL back");
        assert_eq!(actual, 42, "TTL should be 42 after set_ttl(42)");
    }

    #[test]
    fn set_timeout_on_udp_socket_succeeds() {
        let socket = create_udp_socket(false).expect("UDP socket creation failed");
        let result = set_timeout(&socket, Duration::from_millis(500));
        assert!(result.is_ok(), "set_timeout should succeed on UDP socket");
    }

    // Raw ICMP/TCP sockets require root privileges on most systems.
    // These tests verify creation works when running as root.

    #[test]
    #[ignore = "requires root: raw ICMP socket needs CAP_NET_RAW or sudo"]
    fn create_icmp_socket_ipv4_succeeds() {
        let socket = create_icmp_socket(false);
        assert!(socket.is_ok(), "ICMP IPv4 socket should succeed as root");
    }

    #[test]
    #[ignore = "requires root: raw ICMP socket needs CAP_NET_RAW or sudo"]
    fn create_icmp_socket_ipv6_succeeds() {
        let socket = create_icmp_socket(true);
        assert!(socket.is_ok(), "ICMP IPv6 socket should succeed as root");
    }

    #[test]
    #[ignore = "requires root: raw TCP socket needs CAP_NET_RAW or sudo"]
    fn create_tcp_socket_ipv4_succeeds() {
        let socket = create_tcp_socket(false);
        assert!(socket.is_ok(), "TCP IPv4 raw socket should succeed as root");
    }

    #[test]
    #[ignore = "requires root: raw TCP socket needs CAP_NET_RAW or sudo"]
    fn create_tcp_socket_ipv6_succeeds() {
        let socket = create_tcp_socket(true);
        assert!(socket.is_ok(), "TCP IPv6 raw socket should succeed as root");
    }

    #[test]
    #[ignore = "requires root: check_privileges creates a raw ICMP socket"]
    fn check_privileges_succeeds_as_root() {
        assert!(check_privileges().is_ok());
    }
}
