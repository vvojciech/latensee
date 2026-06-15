use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::probe::socket;
use crate::trace::state::ProbeResult;

pub async fn send_tcp_connect_probe(
    target: IpAddr,
    _ttl: u8,
    seq: u16,
    timeout: Duration,
    port: u16,
) -> ProbeResult {
    let timestamp = Instant::now();

    let result = tokio::task::spawn_blocking(move || {
        let ipv6 = target.is_ipv6();
        let sock = socket::create_tcp_connect_socket(ipv6)?;
        socket::set_timeout(&sock, timeout)?;

        let dest: socket2::SockAddr = std::net::SocketAddr::new(target, port).into();
        let send_time = Instant::now();

        match sock.connect_timeout(&dest, timeout) {
            Ok(()) => Ok(Some((send_time.elapsed(), target))),
            Err(e) => {
                // RST (connection refused) still means the target is reachable
                if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    Ok(Some((send_time.elapsed(), target)))
                } else {
                    Ok::<Option<(Duration, IpAddr)>, std::io::Error>(None)
                }
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

    #[tokio::test]
    async fn tcp_connect_to_localhost_returns_target_addr() {
        // Start a TCP listener so connect succeeds
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let target = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let result = send_tcp_connect_probe(target, 1, 1, Duration::from_secs(2), port).await;

        assert_eq!(result.addr, Some(target));
        assert!(result.rtt.is_some());
        assert!(result.rtt.unwrap() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn tcp_connect_refused_still_returns_target_addr() {
        // Port with no listener -- should get ECONNREFUSED (target reachable)
        let target = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let result =
            send_tcp_connect_probe(target, 1, 1, Duration::from_secs(2), 19999).await;

        assert_eq!(result.addr, Some(target));
        assert!(result.rtt.is_some());
    }

    #[tokio::test]
    async fn tcp_connect_timeout_returns_none() {
        // Non-routable address should timeout
        let target = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let result =
            send_tcp_connect_probe(target, 1, 1, Duration::from_millis(200), 80).await;

        assert!(result.rtt.is_none());
        assert!(result.addr.is_none());
    }

    #[tokio::test]
    async fn tcp_connect_ignores_ttl_parameter() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let target = IpAddr::V4(Ipv4Addr::LOCALHOST);
        // TTL=1 would fail with raw sockets on remote targets, but connect ignores it
        let result = send_tcp_connect_probe(target, 1, 1, Duration::from_secs(2), port).await;
        assert!(result.rtt.is_some(), "TTL should not affect TCP connect probe");
    }
}
