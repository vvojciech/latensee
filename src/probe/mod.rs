pub mod icmp;
pub mod socket;
pub mod tcp;

use std::net::IpAddr;
use std::time::{Duration, Instant};

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
}

impl IcmpProbe {
    pub fn new(timeout: Duration, size: u16) -> Self {
        Self {
            timeout,
            size,
            identifier: random(),
        }
    }
}

#[async_trait]
impl Probe for IcmpProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        icmp::send_probe(target, ttl, seq, self.timeout, self.size, self.identifier).await
    }
}

// --- UDP (stub) ---

pub struct UdpProbe {
    pub timeout: Duration,
    pub size: u16,
    pub port: u16,
}

#[async_trait]
impl Probe for UdpProbe {
    async fn send(&self, _target: IpAddr, _ttl: u8, seq: u16) -> ProbeResult {
        ProbeResult {
            seq: seq as u64,
            rtt: None,
            timestamp: Instant::now(),
        }
    }
}

// --- TCP (stub) ---

pub struct TcpProbe {
    pub timeout: Duration,
    pub size: u16,
    pub port: u16,
}

#[async_trait]
impl Probe for TcpProbe {
    async fn send(&self, target: IpAddr, ttl: u8, seq: u16) -> ProbeResult {
        tcp::send_tcp_probe(target, ttl, seq, self.timeout, self.port).await
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
        ProbeProtocol::Udp => Box::new(UdpProbe { timeout, size, port }),
        ProbeProtocol::Tcp => Box::new(TcpProbe { timeout, size, port }),
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
    fn icmp_probe_new_sets_timeout_and_size() {
        let probe = IcmpProbe::new(Duration::from_millis(500), 128);
        assert_eq!(probe.timeout, Duration::from_millis(500));
        assert_eq!(probe.size, 128);
    }

    #[tokio::test]
    async fn udp_probe_stub_returns_none_rtt() {
        let probe = UdpProbe {
            timeout: Duration::from_secs(1),
            size: 64,
            port: 33434,
        };
        let result = probe
            .send(IpAddr::V4(Ipv4Addr::LOCALHOST), 5, 1)
            .await;
        assert!(result.rtt.is_none(), "UDP stub should return None rtt");
        assert_eq!(result.seq, 1);
    }

    #[tokio::test]
    #[ignore = "requires root: TCP probe uses raw sockets"]
    async fn tcp_probe_send_completes() {
        let probe = TcpProbe {
            timeout: Duration::from_secs(1),
            size: 64,
            port: 80,
        };
        let result = probe
            .send(IpAddr::V4(Ipv4Addr::LOCALHOST), 5, 1)
            .await;
        assert_eq!(result.seq, 1);
    }
}
