use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Per-hop latency statistics.
#[derive(Debug, Clone)]
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
}

/// State of a single hop in the trace.
#[derive(Debug, Clone)]
pub struct HopState {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
    pub stats: HopStats,
}

/// Full trace state at a point in time.
#[derive(Debug)]
pub struct TraceState {
    pub target: String,
    pub hops: Vec<HopState>,
    pub round: u64,
    pub started_at: Instant,
}
