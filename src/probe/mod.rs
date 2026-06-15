pub mod icmp;
pub mod socket;
pub mod tcp;
pub mod tcp_connect;
pub mod udp;

use parking_lot::Mutex;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use rand::random;

use crate::config::ProbeProtocol;
use crate::trace::state::ProbeResult;

#[async_trait]
pub trait Probe: Send + Sync {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult;
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

#[async_trait]
impl Probe for IcmpProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        let sock = self.socket.lock().take();
        let (result, returned_sock) =
            icmp::send_probe(target, ttl, seq, self.timeout, self.size, self.identifier, sock)
                .await;
        *self.socket.lock() = returned_sock;
        result
    }
}

// --- UDP ---

pub struct UdpProbe {
    pub timeout: Duration,
    pub port: u16,
    sockets: Mutex<Option<(socket2::Socket, socket2::Socket)>>,
}

#[async_trait]
impl Probe for UdpProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        let socks = self.sockets.lock().take();
        let (result, returned_socks) =
            udp::send_udp_probe(target, ttl, seq, self.timeout, self.port, socks).await;
        *self.sockets.lock() = returned_socks;
        result
    }
}

// --- TCP (stub) ---

pub struct TcpProbe {
    pub timeout: Duration,
    pub port: u16,
    pub port_base: u16,
}

#[async_trait]
impl Probe for TcpProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        tcp::send_tcp_probe(target, ttl, seq, self.timeout, self.port, self.port_base).await
    }
}

// --- TCP Connect (unprivileged) ---

pub struct TcpConnectProbe {
    pub timeout: Duration,
    pub port: u16,
}

#[async_trait]
impl Probe for TcpConnectProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        tcp_connect::send_tcp_connect_probe(target, ttl, seq, self.timeout, self.port).await
    }
}

// --- Factory ---

pub fn create_probe(
    protocol: ProbeProtocol,
    timeout: Duration,
    size: u16,
    port: u16,
) -> Box<dyn Probe> {
    match protocol {
        ProbeProtocol::Icmp => Box::new(IcmpProbe::new(timeout, size)),
        ProbeProtocol::Udp => Box::new(UdpProbe {
            timeout,
            port,
            sockets: Mutex::new(None),
        }),
        ProbeProtocol::Tcp => Box::new(TcpProbe {
            timeout,
            port,
            port_base: random::<u16>() | 0x8000,
        }),
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
        };
        let result = probe
            .send(IpAddr::V4(Ipv4Addr::LOCALHOST), 5, 1)
            .await;

    }
}
