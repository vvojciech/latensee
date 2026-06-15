use std::time::{Duration, Instant};

/// Result of a single probe sent to a target.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub seq: u16,
    pub rtt: Option<Duration>,
    pub timestamp: Instant,
}
