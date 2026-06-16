use std::time::Duration;

use crate::trace::state::HopState;

pub fn format_rtt_ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{:.1}", d.as_secs_f64() * 1000.0),
        None => "-".to_string(),
    }
}

pub fn format_us_to_ms(us: f64) -> String {
    if us == 0.0 {
        "-".to_string()
    } else {
        format!("{:.1}", us / 1000.0)
    }
}

pub fn format_host(hop: &HopState) -> String {
    if let Some(ref hostname) = hop.hostname {
        hostname.clone()
    } else if let Some(addr) = hop.addr {
        addr.to_string()
    } else {
        "???".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::HopStats;
    use std::collections::VecDeque;

    fn make_hop(addr: Option<std::net::IpAddr>, hostname: Option<&str>) -> HopState {
        HopState {
            ttl: 1,
            addr,
            hostname: hostname.map(String::from),
            samples: VecDeque::new(),
            stats: HopStats::new(),
        }
    }

    #[test]
    fn rtt_ms_with_some_duration() {
        assert_eq!(format_rtt_ms(Some(Duration::from_secs_f64(0.0123))), "12.3");
    }

    #[test]
    fn rtt_ms_with_none_returns_dash() {
        assert_eq!(format_rtt_ms(None), "-");
    }

    #[test]
    fn us_to_ms_with_positive_value() {
        assert_eq!(format_us_to_ms(12300.0), "12.3");
    }

    #[test]
    fn us_to_ms_with_zero_returns_dash() {
        assert_eq!(format_us_to_ms(0.0), "-");
    }

    #[test]
    fn host_with_hostname() {
        let hop = make_hop(Some("192.168.1.1".parse().unwrap()), Some("router"));
        assert_eq!(format_host(&hop), "router");
    }

    #[test]
    fn host_with_addr_only() {
        let hop = make_hop(Some("10.0.0.1".parse().unwrap()), None);
        assert_eq!(format_host(&hop), "10.0.0.1");
    }

    #[test]
    fn host_with_nothing_returns_question_marks() {
        let hop = make_hop(None, None);
        assert_eq!(format_host(&hop), "???");
    }
}
