use std::net::IpAddr;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub hostname: String,
    pub addr: IpAddr,
}

#[derive(Debug, Clone)]
pub struct HopStats {
    pub sent: u64,
    pub received: u64,
    pub lost: u64,
    pub loss_pct: f64,
    pub last_rtt: Option<Duration>,
    pub min_rtt: Option<Duration>,
    pub max_rtt: Option<Duration>,
    pub avg_rtt: f64,   // microseconds
    pub jitter: f64,    // microseconds
}

#[derive(Debug, Clone)]
pub struct HopState {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
    pub samples: Vec<Duration>,
    pub stats: HopStats,
}

#[derive(Debug)]
pub struct TraceState {
    pub target: TargetInfo,
    pub hops: Vec<HopState>,
    pub round: u64,
    pub started_at: Instant,
}
