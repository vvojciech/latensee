use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Target host information.
#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub hostname: String,
    pub addr: IpAddr,
}

/// Per-hop latency statistics.
#[derive(Debug, Clone, Default)]
pub struct HopStats {
    pub sent: u64,
    pub received: u64,
    pub lost: u64,
    pub loss_pct: f64,
    pub last_rtt: Option<Duration>,
    pub min_rtt: Option<Duration>,
    pub max_rtt: Option<Duration>,
    /// Average RTT in microseconds.
    pub avg_rtt: f64,
    /// Jitter in microseconds.
    pub jitter: f64,
}

/// State for a single hop in the trace.
#[derive(Debug, Clone)]
pub struct HopState {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
    pub stats: HopStats,
}

/// Full trace state across all hops.
#[derive(Debug, Clone)]
pub struct TraceState {
    pub target: TargetInfo,
    pub hops: Vec<HopState>,
    pub round: u64,
    pub started_at: Instant,
}
