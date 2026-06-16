pub mod icmp;
pub mod socket;
pub mod tcp;
pub mod tcp_connect;
pub mod udp;

use parking_lot::Mutex;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::time::Duration;

use rand::random;

use crate::config::ProbeProtocol;
use crate::trace::state::ProbeResult;

pub trait Probe: Send + Sync {
    fn send(&self, target: IpAddr, ttl: u8, seq: u16)
        -> Pin<Box<dyn Future<Output = ProbeResult> + Send + '_>>;
}

// --- ICMP ---

pub struct IcmpProbe {
    pub timeout: Duration,
    pub size: u16,
    pub identifier: u16,
    socket: Mutex<Option<socket2::Socket>>,
}

impl IcmpProbe {
    pub fn new(timeout: Duration, size: u16) -> Self {
        Self {
            timeout,
            size,
            identifier: random(),
            socket: Mutex::new(None),
        }
    }
}

impl Probe for IcmpProbe {
    fn send(&self, target: IpAddr, ttl: u8, seq: u16)
        -> Pin<Box<dyn Future<Output = ProbeResult> + Send + '_>>
    {
        Box::pin(async move {
            let sock = self.socket.lock().take();
            let (result, returned_sock) =
                icmp::send_probe(target, ttl, seq, self.timeout, self.size, self.identifier, sock)
                    .await;
            *self.socket.lock() = returned_sock;
            result
        })
    }
}

// --- UDP ---

pub struct UdpProbe {
    pub timeout: Duration,
    pub port: u16,
    sockets: Mutex<Option<(socket2::Socket, socket2::Socket)>>,
}

impl Probe for UdpProbe {
    fn send(&self, target: IpAddr, ttl: u8, seq: u16)
        -> Pin<Box<dyn Future<Output = ProbeResult> + Send + '_>>
    {
        Box::pin(async move {
            let socks = self.sockets.lock().take();
            let (result, returned_socks) =
                udp::send_udp_probe(target, ttl, seq, self.timeout, self.port, socks).await;
            *self.sockets.lock() = returned_socks;
            result
        })
    }
}

// --- TCP (stub) ---

pub struct TcpProbe {
    pub timeout: Duration,
    pub port: u16,
    pub port_base: u16,
    pub source_ip: IpAddr,
    sockets: Mutex<Option<(socket2::Socket, socket2::Socket)>>,
}

impl Probe for TcpProbe {
    fn send(&self, target: IpAddr, ttl: u8, seq: u16)
        -> Pin<Box<dyn Future<Output = ProbeResult> + Send + '_>>
    {
        Box::pin(async move {
            let socks = self.sockets.lock().take();
            let (result, returned_socks) =
                tcp::send_tcp_probe(target, ttl, seq, self.timeout, self.port, self.port_base, self.source_ip, socks)
                    .await;
            *self.sockets.lock() = returned_socks;
            result
        })
    }
}

// --- TCP Connect (unprivileged) ---

pub struct TcpConnectProbe {
    pub timeout: Duration,
    pub port: u16,
}

impl Probe for TcpConnectProbe {
    fn send(&self, target: IpAddr, ttl: u8, seq: u16)
        -> Pin<Box<dyn Future<Output = ProbeResult> + Send + '_>>
    {
        Box::pin(async move {
            tcp_connect::send_tcp_connect_probe(target, ttl, seq, self.timeout, self.port).await
        })
    }
}

// --- Factory ---

pub fn create_probe(
    protocol: ProbeProtocol,
    timeout: Duration,
    size: u16,
    port: u16,
    target: IpAddr,
) -> Box<dyn Probe> {
    match protocol {
        ProbeProtocol::Icmp => Box::new(IcmpProbe::new(timeout, size)),
        ProbeProtocol::Udp => Box::new(UdpProbe {
            timeout,
            port,
            sockets: Mutex::new(None),
        }),
        ProbeProtocol::Tcp => {
            let source_ip = tcp::resolve_source_ip(target)
                .unwrap_or(if target.is_ipv6() {
                    IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                } else {
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                });
            Box::new(TcpProbe {
                timeout,
                port,
                port_base: random::<u16>() | 0x8000,
                source_ip,
                sockets: Mutex::new(None),
            })
        }
        ProbeProtocol::TcpConnect => Box::new(TcpConnectProbe { timeout, port }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn create_probe_with_icmp_returns_probe() {
        let probe = create_probe(
            ProbeProtocol::Icmp,
            Duration::from_secs(2),
            64,
            0,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        );
        // Verify it's a valid Probe by checking we got a Box<dyn Probe>
        let _: &dyn Probe = probe.as_ref();
    }

    #[test]
    fn create_probe_with_udp_returns_probe() {
        let probe = create_probe(
            ProbeProtocol::Udp,
            Duration::from_secs(2),
            64,
            33434,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        );
        let _: &dyn Probe = probe.as_ref();
    }

    #[test]
    fn create_probe_with_tcp_returns_probe() {
        let probe = create_probe(
            ProbeProtocol::Tcp,
            Duration::from_secs(2),
            64,
            80,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        );
        let _: &dyn Probe = probe.as_ref();
    }

    #[test]
    fn create_probe_with_tcp_connect_returns_probe() {
        let probe = create_probe(
            ProbeProtocol::TcpConnect,
            Duration::from_secs(2),
            64,
            80,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        );
        let _: &dyn Probe = probe.as_ref();
    }

    #[test]
    fn icmp_probe_new_sets_timeout_and_size() {
        let probe = IcmpProbe::new(Duration::from_millis(500), 128);
        assert_eq!(probe.timeout, Duration::from_millis(500));
        assert_eq!(probe.size, 128);
    }

    #[tokio::test]
    #[ignore = "requires root: UDP probe creates raw ICMP recv socket"]
    async fn udp_probe_send_delegates_to_udp_module() {
        let probe = UdpProbe {
            timeout: Duration::from_secs(1),
            port: 33434,
            sockets: Mutex::new(None),
        };
        let result = probe
            .send(IpAddr::V4(Ipv4Addr::LOCALHOST), 5, 1)
            .await;

    }

    #[tokio::test]
    #[ignore = "requires root: TCP probe uses raw sockets"]
    async fn tcp_probe_send_completes() {
        let probe = TcpProbe {
            timeout: Duration::from_secs(1),
            port: 80,
            port_base: 40000,
            source_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            sockets: Mutex::new(None),
        };
        let result = probe
            .send(IpAddr::V4(Ipv4Addr::LOCALHOST), 5, 1)
            .await;

    }
}
