use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct HopState {
    pub addr: Option<IpAddr>,
    pub hostname: Option<String>,
}

#[derive(Debug, Default)]
pub struct TraceState {
    pub hops: Vec<HopState>,
}
