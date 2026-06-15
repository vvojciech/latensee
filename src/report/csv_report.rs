use std::time::Duration;

use crate::trace::state::TraceState;

/// Format an optional Duration as milliseconds with 3 decimal places.
/// Returns empty string for None.
fn rtt_ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{:.3}", d.as_secs_f64() * 1000.0),
        None => String::new(),
    }
}

/// Format trace state as CSV.
///
/// Header: hop,host,loss_pct,sent,received,last_ms,avg_ms,best_ms,worst_ms,stdev_ms
/// One row per hop. RTTs in ms (3dp). Host falls back: hostname > IP > "???".
pub fn format_csv(state: &TraceState) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());

    wtr.write_record([
        "hop", "host", "loss_pct", "sent", "received", "last_ms", "avg_ms", "best_ms",
        "worst_ms", "stdev_ms",
    ])
    .expect("write header");

    for hop in &state.hops {
        let host = hop
            .hostname
            .as_deref()
            .map(String::from)
            .or_else(|| hop.addr.map(|a| a.to_string()))
            .unwrap_or_else(|| "???".to_string());

        let avg_ms = if hop.stats.received > 0 {
            format!("{:.3}", hop.stats.avg_rtt / 1000.0)
        } else {
            String::new()
        };

        let stdev_ms = if hop.stats.received > 0 {
            format!("{:.3}", hop.stats.jitter / 1000.0)
        } else {
            String::new()
        };

        wtr.write_record([
            &hop.ttl.to_string(),
            &host,
            &format!("{:.1}", hop.stats.loss_pct),
            &hop.stats.sent.to_string(),
            &hop.stats.received.to_string(),
            &rtt_ms(hop.stats.last_rtt),
            &avg_ms,
            &rtt_ms(hop.stats.min_rtt),
            &rtt_ms(hop.stats.max_rtt),
            &stdev_ms,
        ])
        .expect("write row");
    }

    let buf = wtr.into_inner().expect("flush csv");
    String::from_utf8(buf).expect("valid utf8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopState, HopStats, TraceState};
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    fn make_hop(ttl: u8, addr: Option<IpAddr>, hostname: Option<&str>, stats: HopStats) -> HopState {
        HopState {
            ttl,
            addr,
            hostname: hostname.map(String::from),
            stats,
        }
    }

    fn responsive_stats() -> HopStats {
        HopStats {
            sent: 10,
            received: 10,
            lost: 0,
            loss_pct: 0.0,
            last_rtt: Some(Duration::from_micros(12345)),
            min_rtt: Some(Duration::from_micros(10000)),
            max_rtt: Some(Duration::from_micros(15000)),
            avg_rtt: 12000.0,
            jitter: 1500.0,
        }
    }

    fn total_loss_stats() -> HopStats {
        HopStats {
            sent: 10,
            received: 0,
            lost: 10,
            loss_pct: 100.0,
            last_rtt: None,
            min_rtt: None,
            max_rtt: None,
            avg_rtt: 0.0,
            jitter: 0.0,
        }
    }

    fn sample_state(hops: Vec<HopState>) -> TraceState {
        TraceState {
            target: "8.8.8.8".to_string(),
            hops,
            round: 5,
            started_at: Instant::now(),
        }
    }

    #[test]
    fn rtt_ms_some_duration() {
        let d = Duration::from_micros(12345);
        assert_eq!(rtt_ms(Some(d)), "12.345");
    }

    #[test]
    fn rtt_ms_none_returns_empty() {
        assert_eq!(rtt_ms(None), "");
    }

    #[test]
    fn csv_has_correct_header() {
        let state = sample_state(vec![]);
        let csv = format_csv(&state);
        let first_line = csv.lines().next().unwrap();
        assert_eq!(
            first_line,
            "hop,host,loss_pct,sent,received,last_ms,avg_ms,best_ms,worst_ms,stdev_ms"
        );
    }

    #[test]
    fn csv_row_count_matches_hops() {
        let hops = vec![
            make_hop(1, Some("10.0.0.1".parse().unwrap()), Some("gw.local"), responsive_stats()),
            make_hop(2, Some("10.0.0.2".parse().unwrap()), None, responsive_stats()),
        ];
        let state = sample_state(hops);
        let csv = format_csv(&state);
        let lines: Vec<&str> = csv.lines().collect();
        // 1 header + 2 data rows
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn total_loss_hop_has_empty_rtt_fields() {
        let hops = vec![make_hop(1, None, None, total_loss_stats())];
        let state = sample_state(hops);
        let csv = format_csv(&state);
        let data_line = csv.lines().nth(1).unwrap();
        let fields: Vec<&str> = data_line.split(',').collect();
        // hop=1, host=???, loss_pct=100.0, sent=10, received=0,
        // last_ms="", avg_ms="", best_ms="", worst_ms="", stdev_ms=""
        assert_eq!(fields[0], "1");
        assert_eq!(fields[1], "???");
        assert_eq!(fields[2], "100.0");
        // RTT fields (indices 5-9) should all be empty
        for i in 5..=9 {
            assert_eq!(fields[i], "", "field index {} should be empty", i);
        }
    }
}
