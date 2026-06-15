use std::net::IpAddr;
use std::time::Duration;

/// Result of a single probe to a hop.
#[derive(Debug, Clone)]
pub enum ProbeResult {
    Reply {
        addr: IpAddr,
        rtt: Duration,
        hostname: Option<String>,
    },
    Timeout,
}

/// Rolling statistics for a single hop.
#[derive(Debug, Clone, Default)]
pub struct HopStats {
    pub sent: u64,
    pub received: u64,
    pub last_rtt: Option<Duration>,
    pub min_rtt: Option<Duration>,
    pub max_rtt: Option<Duration>,
    pub avg_rtt: Option<Duration>,
    pub jitter: Option<Duration>,
    pub recent_rtts: Vec<Duration>,
}

/// State of a single hop in the trace.
#[derive(Debug, Clone)]
pub struct HopState {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
    pub stats: HopStats,
}

/// Shared state for the entire trace, updated by the probe engine
/// and read by the TUI.
#[derive(Debug)]
pub struct TraceState {
    pub target: String,
    pub target_addr: Option<IpAddr>,
    pub hops: Vec<HopState>,
    pub round: u64,
    pub max_ttl: u8,
}

impl TraceState {
    pub fn new(target: String, max_ttl: u8) -> Self {
        Self {
            target,
            target_addr: None,
            hops: Vec::new(),
            round: 0,
            max_ttl,
        }
    }

    pub fn hop_count(&self) -> usize {
        self.hops.len()
    }
}
