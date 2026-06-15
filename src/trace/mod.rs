pub mod dns;
pub mod state;

use parking_lot::RwLock;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::probe::{create_probe, Probe};
use crate::trace::state::TraceState;

const DEFAULT_MAX_SAMPLES: usize = 300;

pub struct TraceEngine {
    state: Arc<RwLock<TraceState>>,
    target: IpAddr,
    probe: Box<dyn Probe>,
    interval: Duration,
    max_hops: u8,
    count: Option<u64>,
    max_samples: usize,
}

impl TraceEngine {
    pub fn new(state: Arc<RwLock<TraceState>>, config: &Config) -> Self {
        let target = state.read().target.addr;
        let probe = create_probe(
            config.protocol,
            Duration::from_secs_f64(config.timeout),
            config.size,
            config.port,
        );
        Self {
            state,
            target,
            probe,
            interval: Duration::from_secs_f64(config.interval),
            max_hops: config.max_hops,
            count: config.count,
            max_samples: DEFAULT_MAX_SAMPLES,
        }
    }

    pub async fn run(&self, cancel: CancellationToken) {
        let mut round: u64 = 0;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            if let Some(limit) = self.count {
                if round >= limit {
                    break;
                }
            }

            self.probe_round(round).await;

            {
                let mut state = self.state.write();
                state.round = round + 1;
            }

            round += 1;

            tokio::select! {
                _ = tokio::time::sleep(self.interval) => {}
                _ = cancel.cancelled() => { break; }
            }
        }
    }

    async fn probe_round(&self, round: u64) {
        // Send probes sequentially (each has internal timeout).
        // Concurrent sending requires Probe to be shareable across tasks,
        // which needs a different design. Sequential is correct for now.
        let mut results = Vec::with_capacity(self.max_hops as usize);

        for ttl in 1..=self.max_hops {
            let seq = (round * self.max_hops as u64 + ttl as u64) as u16;
            let result = self.probe.send(self.target, ttl, seq).await;
            let reached = result.addr == Some(self.target);
            results.push((ttl, result));
            if reached {
                break;
            }
        }

        let mut state = self.state.write();
        for (ttl, result) in results {
            state.ensure_hop(ttl);
            let hop = &mut state.hops[(ttl - 1) as usize];
            hop.add_probe(result, self.max_samples);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProbeProtocol;
    use crate::trace::state::{ProbeResult, TargetInfo, TraceState};
    use async_trait::async_trait;
    use std::net::Ipv4Addr;

    struct MockProbe {
        rtt: Option<Duration>,
    }

    #[async_trait]
    impl Probe for MockProbe {
        async fn send(&self, _target: IpAddr, _ttl: u8, _seq: u16) -> ProbeResult {
            ProbeResult {
                rtt: self.rtt,
                addr: None,
                error: None,
            }
        }
    }

    struct MockProbeWithRoute {
        target: IpAddr,
        route_len: u8,
    }

    #[async_trait]
    impl Probe for MockProbeWithRoute {
        async fn send(&self, _target: IpAddr, ttl: u8, _seq: u16) -> ProbeResult {
            let addr = if ttl >= self.route_len {
                Some(self.target)
            } else {
                Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, ttl)))
            };
            ProbeResult {
                rtt: Some(Duration::from_millis(ttl as u64)),
                addr,
                error: None,
            }
        }
    }

    fn test_config() -> Config {
        Config {
            targets: vec!["192.0.2.1".to_string()],
            interval: 1.0,
            max_hops: 3,
            count: Some(1),
            size: 64,
            timeout: 2.0,
            protocol: ProbeProtocol::Icmp,
            port: 0,
            report: false,
            csv: false,
            json: false,
            no_dns: true,
            ip_version: crate::config::IpVersion::Auto,
        }
    }

    fn test_state() -> Arc<RwLock<TraceState>> {
        let target = TargetInfo {
            hostname: "192.0.2.1".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        };
        Arc::new(RwLock::new(TraceState::new(target, 3)))
    }

    #[test]
    fn new_creates_engine_with_correct_config() {
        let state = test_state();
        let config = test_config();
        let engine = TraceEngine::new(state, &config);

        assert_eq!(engine.max_hops, 3);
        assert_eq!(engine.count, Some(1));
        assert_eq!(engine.interval, Duration::from_secs(1));
        assert_eq!(engine.max_samples, DEFAULT_MAX_SAMPLES);
        assert_eq!(engine.target, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
    }

    #[test]
    fn new_with_unlimited_count() {
        let state = test_state();
        let mut config = test_config();
        config.count = None;
        let engine = TraceEngine::new(state, &config);

        assert_eq!(engine.count, None);
    }

    #[tokio::test]
    async fn probe_round_updates_state_for_all_hops() {
        let state = test_state();
        let rtt = Duration::from_millis(10);

        let engine = TraceEngine {
            state: state.clone(),
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            probe: Box::new(MockProbe { rtt: Some(rtt) }),
            interval: Duration::from_secs(1),
            max_hops: 3,
            count: Some(1),
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        engine.probe_round(0).await;

        let s = state.read();
        assert_eq!(s.hops.len(), 3);
        for (i, hop) in s.hops.iter().enumerate() {
            assert_eq!(hop.ttl, (i + 1) as u8);
            assert_eq!(hop.stats.sent, 1);
            assert_eq!(hop.stats.received, 1);
            assert_eq!(hop.samples.len(), 1);
            assert_eq!(hop.samples[0].rtt, Some(rtt));
        }
    }

    #[tokio::test]
    async fn probe_round_records_timeouts() {
        let state = test_state();

        let engine = TraceEngine {
            state: state.clone(),
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            probe: Box::new(MockProbe { rtt: None }),
            interval: Duration::from_secs(1),
            max_hops: 2,
            count: Some(1),
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        engine.probe_round(0).await;

        let s = state.read();
        assert_eq!(s.hops.len(), 2);
        for hop in &s.hops {
            assert_eq!(hop.stats.sent, 1);
            assert_eq!(hop.stats.lost, 1);
            assert!(hop.samples[0].rtt.is_none());
        }
    }

    #[tokio::test]
    async fn run_stops_after_count_rounds() {
        let state = test_state();

        let engine = TraceEngine {
            state: state.clone(),
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            probe: Box::new(MockProbe {
                rtt: Some(Duration::from_millis(1)),
            }),
            interval: Duration::from_millis(1),
            max_hops: 2,
            count: Some(3),
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        let cancel = CancellationToken::new();
        engine.run(cancel).await;

        let s = state.read();
        assert_eq!(s.round, 3);
        for hop in &s.hops {
            assert_eq!(hop.stats.sent, 3);
        }
    }

    #[tokio::test]
    async fn run_stops_on_cancellation() {
        let state = test_state();

        let engine = TraceEngine {
            state: state.clone(),
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            probe: Box::new(MockProbe {
                rtt: Some(Duration::from_millis(1)),
            }),
            interval: Duration::from_secs(60),
            max_hops: 2,
            count: None,
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
        });

        engine.run(cancel).await;

        let s = state.read();
        assert!(s.round >= 1);
    }

    #[tokio::test]
    async fn probe_round_stops_at_destination() {
        let target_ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let target = TargetInfo {
            hostname: "192.0.2.1".to_string(),
            addr: target_ip,
        };
        let state = Arc::new(RwLock::new(TraceState::new(target, 5)));

        let engine = TraceEngine {
            state: state.clone(),
            target: target_ip,
            probe: Box::new(MockProbeWithRoute {
                target: target_ip,
                route_len: 3,
            }),
            interval: Duration::from_secs(1),
            max_hops: 5,
            count: Some(1),
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        engine.probe_round(0).await;

        let s = state.read();
        assert_eq!(s.hops.len(), 3, "should stop at hop 3 (the destination)");
        assert_eq!(s.hops[0].addr, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert_eq!(s.hops[1].addr, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
        assert_eq!(s.hops[2].addr, Some(target_ip));
    }

    #[tokio::test]
    async fn probe_round_probes_all_hops_when_target_unreachable() {
        let state = test_state();

        let engine = TraceEngine {
            state: state.clone(),
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            probe: Box::new(MockProbe { rtt: None }),
            interval: Duration::from_secs(1),
            max_hops: 3,
            count: Some(1),
            max_samples: DEFAULT_MAX_SAMPLES,
        };

        engine.probe_round(0).await;

        let s = state.read();
        assert_eq!(s.hops.len(), 3, "should probe all hops when target never responds");
    }

    #[test]
    fn lock_survives_panicked_writer() {
        let target = TargetInfo {
            hostname: "192.0.2.1".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        };
        let state = Arc::new(RwLock::new(TraceState::new(target, 3)));
        let state_clone = state.clone();

        let handle = std::thread::spawn(move || {
            let _guard = state_clone.write();
            panic!("simulated probe panic while holding write lock");
        });

        let _ = handle.join(); // thread panicked, lock is now poisoned with std RwLock

        // With std::sync::RwLock this panics (poisoned lock).
        // With parking_lot::RwLock this succeeds (no poisoning).
        let s = state.read();
        assert_eq!(s.hop_count(), 0);
    }
}
