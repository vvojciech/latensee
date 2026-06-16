use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub struct TraceState {
    pub target: TargetInfo,
    pub hops: Vec<HopState>,
    pub round: u64,
    pub started_at: Instant,
}

#[derive(Clone)]
pub struct TargetInfo {
    pub hostname: String,
    pub addr: IpAddr,
}

impl std::fmt::Display for TargetInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.hostname, self.addr)
    }
}

pub struct HopState {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
    pub samples: VecDeque<ProbeResult>,
    pub stats: HopStats,
}

#[derive(Default)]
pub struct HopStats {
    pub sent: u64,
    pub received: u64,
    pub lost: u64,
    pub loss_pct: f64,
    pub last_rtt: Option<Duration>,
    pub min_rtt: Option<Duration>,
    pub max_rtt: Option<Duration>,
    pub avg_rtt: f64,
    pub jitter: f64,
    pub errors: u64,
}

pub struct ProbeResult {
    pub rtt: Option<Duration>,
    pub addr: Option<IpAddr>,
    pub error: Option<String>,
}

impl HopStats {
    pub fn new() -> Self {
        Self {
            sent: 0,
            received: 0,
            lost: 0,
            loss_pct: 0.0,
            last_rtt: None,
            min_rtt: None,
            max_rtt: None,
            avg_rtt: 0.0,
            jitter: 0.0,
            errors: 0,
        }
    }

    pub fn record_probe(&mut self, result: &ProbeResult) {
        self.sent += 1;
        if result.error.is_some() {
            self.errors += 1;
        }

        match result.rtt {
            Some(rtt) => {
                self.received += 1;
                self.last_rtt = Some(rtt);

                let rtt_us = rtt.as_micros() as f64;

                // Min/max
                self.min_rtt = Some(match self.min_rtt {
                    Some(prev) => prev.min(rtt),
                    None => rtt,
                });
                self.max_rtt = Some(match self.max_rtt {
                    Some(prev) => prev.max(rtt),
                    None => rtt,
                });

                // Welford's online algorithm for mean and variance
                let old_avg = self.avg_rtt;
                self.avg_rtt += (rtt_us - old_avg) / self.received as f64;
                // m2 accumulates sum of squared differences
                // We store jitter as population std dev, so we need to track m2
                // m2_old = jitter_old^2 * (received-1)
                let m2_old = if self.received > 1 {
                    self.jitter * self.jitter * (self.received - 1) as f64
                } else {
                    0.0
                };
                let m2_new = m2_old + (rtt_us - old_avg) * (rtt_us - self.avg_rtt);
                self.jitter = (m2_new / self.received as f64).sqrt();
            }
            None => {
                self.lost += 1;
                self.last_rtt = None;
            }
        }

        self.loss_pct = (self.lost as f64 / self.sent as f64) * 100.0;
    }
}

impl HopState {
    pub fn new(ttl: u8) -> Self {
        Self {
            ttl,
            addr: None,
            hostname: None,
            samples: VecDeque::new(),
            stats: HopStats::new(),
        }
    }

    pub fn add_probe(&mut self, result: ProbeResult, max_samples: usize) {
        if let Some(new_addr) = result.addr {
            if self.addr != Some(new_addr) {
                self.addr = Some(new_addr);
                self.hostname = None;
            }
        }
        self.stats.record_probe(&result);
        self.samples.push_back(result);
        while self.samples.len() > max_samples {
            self.samples.pop_front();
        }
    }

    pub fn reset(&mut self) {
        self.stats = HopStats::new();
        self.samples.clear();
    }
}

impl TraceState {
    pub fn new(target: TargetInfo, _max_hops: u8) -> Self {
        Self {
            target,
            hops: Vec::new(),
            round: 0,
            started_at: Instant::now(),
        }
    }

    pub fn reset_all(&mut self) {
        for hop in &mut self.hops {
            hop.reset();
        }
        self.round = 0;
    }

    pub fn ensure_hop(&mut self, ttl: u8) {
        while self.hops.len() < ttl as usize {
            let next_ttl = (self.hops.len() + 1) as u8;
            self.hops.push(HopState::new(next_ttl));
        }
    }

    pub fn hop_count(&self) -> usize {
        self.hops.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn make_probe(rtt_us: Option<u64>) -> ProbeResult {
        ProbeResult {
            rtt: rtt_us.map(Duration::from_micros),
            addr: None,
            error: None,
        }
    }

    #[test]
    fn hop_stats_starts_at_zero() {
        let stats = HopStats::new();
        assert_eq!(stats.sent, 0);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.lost, 0);
        assert_eq!(stats.loss_pct, 0.0);
        assert!(stats.last_rtt.is_none());
        assert!(stats.min_rtt.is_none());
        assert!(stats.max_rtt.is_none());
        assert_eq!(stats.avg_rtt, 0.0);
        assert_eq!(stats.jitter, 0.0);
    }

    #[test]
    fn record_successful_probe_updates_stats() {
        let mut stats = HopStats::new();
        let probe = make_probe(Some(5000));

        stats.record_probe(&probe);

        assert_eq!(stats.sent, 1);
        assert_eq!(stats.received, 1);
        assert_eq!(stats.lost, 0);
        assert_eq!(stats.loss_pct, 0.0);
        assert_eq!(stats.last_rtt, Some(Duration::from_micros(5000)));
        assert_eq!(stats.min_rtt, Some(Duration::from_micros(5000)));
        assert_eq!(stats.max_rtt, Some(Duration::from_micros(5000)));
        assert_eq!(stats.avg_rtt, 5000.0);
    }

    #[test]
    fn record_timeout_increments_sent_and_lost() {
        let mut stats = HopStats::new();
        let probe = make_probe(None);

        stats.record_probe(&probe);

        assert_eq!(stats.sent, 1);
        assert_eq!(stats.received, 0);
        assert_eq!(stats.lost, 1);
        assert_eq!(stats.loss_pct, 100.0);
        assert!(stats.last_rtt.is_none());
        assert!(stats.min_rtt.is_none());
        assert!(stats.max_rtt.is_none());
        assert_eq!(stats.avg_rtt, 0.0);
    }

    #[test]
    fn welford_jitter_with_known_values() {
        let mut stats = HopStats::new();
        // Values: 10, 20, 30 ms -> std dev = sqrt(((10-20)^2 + (20-20)^2 + (30-20)^2) / 3)
        // = sqrt((100 + 0 + 100) / 3) = sqrt(66.666...) ~= 8164.965... microseconds
        let values_us = [10_000u64, 20_000, 30_000];
        for (i, &v) in values_us.iter().enumerate() {
            stats.record_probe(&make_probe(Some(v)));
        }

        let expected_avg = 20_000.0;
        assert!((stats.avg_rtt - expected_avg).abs() < 0.01);

        // Population std dev = sqrt(variance / n)
        // Welford's M2/n = population variance
        let expected_jitter = (200_000_000.0_f64 / 3.0).sqrt(); // ~8164.97
        assert!(
            (stats.jitter - expected_jitter).abs() < 0.01,
            "jitter {} != expected {}",
            stats.jitter,
            expected_jitter
        );
    }

    #[test]
    fn ring_buffer_trims_to_max_samples() {
        let mut hop = HopState::new(3);
        let max = 5;
        for _ in 0..10u64 {
            hop.add_probe(make_probe(Some(1000)), max);
        }

        assert_eq!(hop.samples.len(), max);
        assert_eq!(hop.stats.sent, 10);
    }

    #[test]
    fn loss_pct_after_mixed_results() {
        let mut stats = HopStats::new();
        // 3 successes, 2 timeouts = 40% loss
        stats.record_probe(&make_probe(Some(1000)));
        stats.record_probe(&make_probe(None));
        stats.record_probe(&make_probe(Some(2000)));
        stats.record_probe(&make_probe(None));
        stats.record_probe(&make_probe(Some(3000)));

        assert_eq!(stats.sent, 5);
        assert_eq!(stats.received, 3);
        assert_eq!(stats.lost, 2);
        assert!((stats.loss_pct - 40.0).abs() < 0.01);
    }

    #[test]
    fn min_max_rtt_tracked_correctly() {
        let mut stats = HopStats::new();
        stats.record_probe(&make_probe(Some(5000)));
        stats.record_probe(&make_probe(Some(1000)));
        stats.record_probe(&make_probe(Some(9000)));
        stats.record_probe(&make_probe(None)); // timeout should not affect min/max

        assert_eq!(stats.min_rtt, Some(Duration::from_micros(1000)));
        assert_eq!(stats.max_rtt, Some(Duration::from_micros(9000)));
        assert_eq!(stats.last_rtt, None); // last probe was a timeout
    }

    #[test]
    fn record_probe_with_error_increments_error_count() {
        let mut stats = HopStats::new();
        let probe = ProbeResult {
            rtt: None,
            addr: None,
            error: Some("permission denied".into()),
        };
        stats.record_probe(&probe);

        assert_eq!(stats.errors, 1);
        assert_eq!(stats.lost, 1);
        assert_eq!(stats.sent, 1);
    }

    #[test]
    fn record_probe_timeout_has_no_error() {
        let mut stats = HopStats::new();
        stats.record_probe(&make_probe(None));

        assert_eq!(stats.errors, 0);
        assert_eq!(stats.lost, 1);
    }

    #[test]
    fn add_probe_updates_addr_on_change() {
        let mut hop = HopState::new(3);
        let addr_a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let addr_b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        hop.add_probe(ProbeResult { rtt: Some(Duration::from_millis(5)), addr: Some(addr_a), error: None }, 10);
        assert_eq!(hop.addr, Some(addr_a));

        hop.add_probe(ProbeResult { rtt: Some(Duration::from_millis(6)), addr: Some(addr_b), error: None }, 10);
        assert_eq!(hop.addr, Some(addr_b), "addr should update when ECMP router changes");
        assert!(hop.hostname.is_none(), "hostname should be cleared on addr change");
    }

    #[test]
    fn add_probe_clears_hostname_on_addr_change() {
        let mut hop = HopState::new(3);
        let addr_a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let addr_b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        hop.add_probe(ProbeResult { rtt: Some(Duration::from_millis(5)), addr: Some(addr_a), error: None }, 10);
        hop.hostname = Some("router-a.example.com".into());

        hop.add_probe(ProbeResult { rtt: Some(Duration::from_millis(6)), addr: Some(addr_b), error: None }, 10);
        assert!(hop.hostname.is_none(), "hostname should be cleared when addr changes");
    }

    #[test]
    fn add_probe_timeout_does_not_clear_addr() {
        let mut hop = HopState::new(3);
        let addr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        hop.add_probe(ProbeResult { rtt: Some(Duration::from_millis(5)), addr: Some(addr), error: None }, 10);
        hop.hostname = Some("router.example.com".into());

        hop.add_probe(ProbeResult { rtt: None, addr: None, error: None }, 10);
        assert_eq!(hop.addr, Some(addr), "timeout should not clear addr");
        assert_eq!(hop.hostname.as_deref(), Some("router.example.com"), "timeout should not clear hostname");
    }

    #[test]
    fn reset_clears_stats_and_samples() {
        let mut hop = HopState::new(3);
        let addr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        hop.add_probe(ProbeResult { rtt: Some(Duration::from_micros(5000)), addr: Some(addr), error: None }, 10);
        hop.add_probe(ProbeResult { rtt: Some(Duration::from_micros(6000)), addr: Some(addr), error: None }, 10);
        assert_eq!(hop.stats.sent, 2);
        assert_eq!(hop.samples.len(), 2);

        hop.reset();
        assert_eq!(hop.stats.sent, 0);
        assert_eq!(hop.stats.received, 0);
        assert!(hop.samples.is_empty());
        assert_eq!(hop.addr, Some(addr), "reset should preserve addr");
    }

    #[test]
    fn ensure_hop_grows_vec() {
        let target = TargetInfo {
            hostname: "example.com".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        };
        let mut state = TraceState::new(target, 30);
        assert!(state.hops.is_empty());

        state.ensure_hop(5);
        assert_eq!(state.hops.len(), 5);
        // Each hop should have sequential TTL
        for (i, hop) in state.hops.iter().enumerate() {
            assert_eq!(hop.ttl, (i + 1) as u8);
        }

        // Calling again with smaller TTL should not shrink
        state.ensure_hop(3);
        assert_eq!(state.hops.len(), 5);

        // Calling with larger TTL should grow
        state.ensure_hop(8);
        assert_eq!(state.hops.len(), 8);
        assert_eq!(state.hops[7].ttl, 8);
    }
}
