use serde::Serialize;
use std::time::Duration;

use crate::trace::state::{TraceState, HopState};

/// JSON-serializable report for a completed trace.
#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub target: String,
    pub target_ip: String,
    pub rounds: u64,
    pub hops: Vec<JsonHop>,
}

/// JSON-serializable hop entry.
#[derive(Debug, Serialize)]
pub struct JsonHop {
    pub ttl: u8,
    pub host: Option<String>,
    pub ip: Option<String>,
    pub loss_pct: f64,
    pub sent: u64,
    pub received: u64,
    pub last_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub best_ms: Option<f64>,
    pub worst_ms: Option<f64>,
    pub stdev_ms: Option<f64>,
}

/// Convert a Duration to milliseconds as f64.
fn duration_to_ms(d: &Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Convert microseconds to milliseconds.
fn us_to_ms(us: f64) -> f64 {
    us / 1000.0
}

/// Build a JsonReport from internal TraceState.
pub fn build_report(state: &TraceState) -> JsonReport {
    let hops = state.hops.iter().map(|hop| build_hop(hop)).collect();

    JsonReport {
        target: state.target.hostname.clone(),
        target_ip: state.target.addr.to_string(),
        rounds: state.round,
        hops,
    }
}

fn build_hop(hop: &HopState) -> JsonHop {
    let stats = &hop.stats;
    let has_data = stats.received > 0;

    JsonHop {
        ttl: hop.ttl,
        host: hop.hostname.clone(),
        ip: hop.addr.map(|a| a.to_string()),
        loss_pct: stats.loss_pct,
        sent: stats.sent,
        received: stats.received,
        last_ms: if has_data { stats.last_rtt.as_ref().map(duration_to_ms) } else { None },
        avg_ms: if has_data { Some(us_to_ms(stats.avg_rtt)) } else { None },
        best_ms: if has_data { stats.min_rtt.as_ref().map(duration_to_ms) } else { None },
        worst_ms: if has_data { stats.max_rtt.as_ref().map(duration_to_ms) } else { None },
        stdev_ms: if has_data { Some(us_to_ms(stats.jitter)) } else { None },
    }
}

/// Produce pretty-printed JSON from TraceState.
pub fn format_json(state: &TraceState) -> String {
    let report = build_report(state);
    serde_json::to_string_pretty(&report).expect("JsonReport serialization should not fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopStats, HopState, TargetInfo, TraceState};
    use std::collections::VecDeque;
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    fn make_target() -> TargetInfo {
        TargetInfo {
            hostname: "example.com".to_string(),
            addr: "93.184.216.34".parse::<IpAddr>().unwrap(),
        }
    }

    fn make_state(hops: Vec<HopState>, round: u64) -> TraceState {
        TraceState {
            target: make_target(),
            hops,
            round,
            started_at: Instant::now(),
        }
    }

    fn hop_with_data() -> HopState {
        HopState {
            ttl: 3,
            addr: Some("10.0.0.1".parse().unwrap()),
            hostname: Some("router.local".to_string()),
            samples: VecDeque::new(),
            stats: HopStats {
                sent: 10,
                received: 9,
                lost: 1,
                loss_pct: 10.0,
                last_rtt: Some(Duration::from_micros(12_500)),
                min_rtt: Some(Duration::from_micros(8_000)),
                max_rtt: Some(Duration::from_micros(20_000)),
                avg_rtt: 13_500.0, // microseconds
                jitter: 2_100.0,   // microseconds
                errors: 0,
            },
        }
    }

    fn hop_no_addr() -> HopState {
        HopState {
            ttl: 2,
            addr: None,
            hostname: None,
            samples: VecDeque::new(),
            stats: HopStats {
                sent: 5,
                received: 0,
                lost: 5,
                loss_pct: 100.0,
                ..Default::default()
            },
        }
    }

    fn hop_total_loss() -> HopState {
        HopState {
            ttl: 4,
            addr: Some("10.0.0.2".parse().unwrap()),
            hostname: None,
            samples: VecDeque::new(),
            stats: HopStats {
                sent: 8,
                received: 0,
                lost: 8,
                loss_pct: 100.0,
                last_rtt: None,
                min_rtt: None,
                max_rtt: None,
                avg_rtt: 0.0,
                jitter: 0.0, errors: 0,
            },
        }
    }

    #[test]
    fn build_report_sets_target_and_rounds() {
        let state = make_state(vec![], 42);
        let report = build_report(&state);

        assert_eq!(report.target, "example.com");
        assert_eq!(report.target_ip, "93.184.216.34");
        assert_eq!(report.rounds, 42);
        assert!(report.hops.is_empty());
    }

    #[test]
    fn build_report_converts_hop_rtt_to_ms() {
        let state = make_state(vec![hop_with_data()], 5);
        let report = build_report(&state);
        let hop = &report.hops[0];

        assert_eq!(hop.ttl, 3);
        assert_eq!(hop.ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(hop.host.as_deref(), Some("router.local"));
        assert_eq!(hop.sent, 10);
        assert_eq!(hop.received, 9);
        assert!((hop.loss_pct - 10.0).abs() < f64::EPSILON);

        // Duration(12_500us) -> 12.5ms
        assert!((hop.last_ms.unwrap() - 12.5).abs() < 0.001);
        // Duration(8_000us) -> 8.0ms
        assert!((hop.best_ms.unwrap() - 8.0).abs() < 0.001);
        // Duration(20_000us) -> 20.0ms
        assert!((hop.worst_ms.unwrap() - 20.0).abs() < 0.001);
        // 13_500us -> 13.5ms
        assert!((hop.avg_ms.unwrap() - 13.5).abs() < 0.001);
        // 2_100us -> 2.1ms
        assert!((hop.stdev_ms.unwrap() - 2.1).abs() < 0.001);
    }

    #[test]
    fn build_report_hop_no_addr_has_none_ip_and_host() {
        let state = make_state(vec![hop_no_addr()], 1);
        let report = build_report(&state);
        let hop = &report.hops[0];

        assert_eq!(hop.ttl, 2);
        assert!(hop.ip.is_none());
        assert!(hop.host.is_none());
    }

    #[test]
    fn build_report_hop_total_loss_has_none_rtts() {
        let state = make_state(vec![hop_total_loss()], 1);
        let report = build_report(&state);
        let hop = &report.hops[0];

        assert_eq!(hop.sent, 8);
        assert_eq!(hop.received, 0);
        assert!((hop.loss_pct - 100.0).abs() < f64::EPSILON);
        assert!(hop.last_ms.is_none());
        assert!(hop.avg_ms.is_none());
        assert!(hop.best_ms.is_none());
        assert!(hop.worst_ms.is_none());
        assert!(hop.stdev_ms.is_none());
    }

    #[test]
    fn format_json_produces_valid_json() {
        let state = make_state(vec![hop_with_data(), hop_no_addr()], 10);
        let json_str = format_json(&state);

        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("format_json output must be valid JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn json_contains_expected_field_names() {
        let state = make_state(vec![hop_with_data()], 3);
        let json_str = format_json(&state);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Top-level fields
        assert!(parsed.get("target").is_some());
        assert!(parsed.get("target_ip").is_some());
        assert!(parsed.get("rounds").is_some());
        assert!(parsed.get("hops").is_some());

        // Hop fields
        let hop = &parsed["hops"][0];
        for field in &[
            "ttl", "host", "ip", "loss_pct", "sent", "received",
            "last_ms", "avg_ms", "best_ms", "worst_ms", "stdev_ms",
        ] {
            assert!(hop.get(field).is_some(), "missing field: {field}");
        }
    }
}
