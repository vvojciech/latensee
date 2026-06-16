use std::fmt::Write;

use crate::trace::state::TraceState;

use super::format::{format_host, format_rtt_ms, format_us_to_ms};

pub fn format_report(state: &TraceState) -> String {
    let mut out = String::new();

    writeln!(
        out,
        "latensee report for {} ({}) - {} rounds",
        state.target.hostname, state.target.addr, state.round
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "{:>2}  {:<28} {:>5}  {:>5} {:>5} {:>5}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}",
        "#", "Host", "Loss%", "Sent", "Rcvd", "Errs", "Last", "Avg", "Best", "Wrst", "StDev"
    )
    .unwrap();

    for hop in &state.hops {
        let host = format_host(hop);
        let loss = format!("{:.1}%", hop.stats.loss_pct);
        let last = format_rtt_ms(hop.stats.last_rtt);
        let avg = format_us_to_ms(hop.stats.avg_rtt);
        let best = format_rtt_ms(hop.stats.min_rtt);
        let wrst = format_rtt_ms(hop.stats.max_rtt);
        let stdev = format_us_to_ms(hop.stats.jitter);
        let errs = if hop.stats.errors > 0 {
            hop.stats.errors.to_string()
        } else {
            "-".to_string()
        };

        writeln!(
            out,
            "{:>2}  {:<28} {:>5}  {:>5} {:>5} {:>5}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}",
            hop.ttl, host, loss, hop.stats.sent, hop.stats.received, errs, last, avg, best, wrst, stdev
        )
        .unwrap();
    }

    out
}

pub fn print_report(state: &TraceState) {
    print!("{}", format_report(state));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopState, HopStats, TargetInfo, TraceState};
    use std::collections::VecDeque;
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    fn dur_ms(ms: f64) -> Duration {
        Duration::from_secs_f64(ms / 1000.0)
    }

    fn make_hop(
        ttl: u8,
        addr: Option<IpAddr>,
        hostname: Option<&str>,
        sent: u64,
        received: u64,
        loss_pct: f64,
        last_ms: Option<f64>,
        avg_us: f64,
        min_ms: Option<f64>,
        max_ms: Option<f64>,
        jitter_us: f64,
    ) -> HopState {
        HopState {
            ttl,
            addr,
            hostname: hostname.map(String::from),
            samples: VecDeque::new(),
            stats: HopStats {
                sent,
                received,
                lost: sent - received,
                loss_pct,
                last_rtt: last_ms.map(dur_ms),
                min_rtt: min_ms.map(dur_ms),
                max_rtt: max_ms.map(dur_ms),
                avg_rtt: avg_us,
                jitter: jitter_us, errors: 0,
            },
        }
    }

    fn sample_state() -> TraceState {
        let target = TargetInfo {
            hostname: "example.com".to_string(),
            addr: "93.184.216.34".parse().unwrap(),
        };

        let hops = vec![
            make_hop(
                1,
                Some("192.168.1.1".parse().unwrap()),
                Some("router"),
                47, 47, 0.0,
                Some(1.2), 1100.0, Some(0.8), Some(2.3), 300.0,
            ),
            make_hop(
                2,
                Some("10.0.0.1".parse().unwrap()),
                None,
                47, 47, 0.0,
                Some(8.4), 8200.0, Some(7.1), Some(12.3), 1100.0,
            ),
            make_hop(
                3, None, None,
                47, 0, 100.0,
                None, 0.0, None, None, 0.0,
            ),
            make_hop(
                4,
                Some("142.250.180.14".parse().unwrap()),
                None,
                47, 46, 2.1,
                Some(12.1), 11500.0, Some(10.2), Some(15.3), 1200.0,
            ),
        ];

        TraceState {
            target,
            hops,
            round: 47,
            started_at: Instant::now(),
        }
    }

    #[test]
    fn format_report_contains_header() {
        let state = sample_state();
        let report = format_report(&state);
        assert!(report.contains("latensee report for example.com (93.184.216.34) - 47 rounds"));
        assert!(report.contains("Host"));
        assert!(report.contains("Loss%"));
        assert!(report.contains("Sent"));
        assert!(report.contains("Rcvd"));
        assert!(report.contains("Last"));
        assert!(report.contains("Avg"));
        assert!(report.contains("Best"));
        assert!(report.contains("Wrst"));
        assert!(report.contains("StDev"));
    }

    #[test]
    fn format_report_full_loss_hop_shows_dashes() {
        let state = sample_state();
        let report = format_report(&state);
        let lines: Vec<&str> = report.lines().collect();
        // Hop 3 is the 100% loss hop (line index: 0=header, 1=blank, 2=column headers, 3=hop1, 4=hop2, 5=hop3)
        let loss_line = lines[5];
        assert!(loss_line.contains("???"), "should show ??? for unknown host");
        assert!(loss_line.contains("100.0%"), "should show 100.0% loss");
        // Count dashes for RTT fields (Last, Avg, Best, Wrst, StDev)
        let dash_count = loss_line.matches("    -").count();
        assert!(dash_count >= 5, "all RTT columns should be dashes, found {dash_count}");
    }

    #[test]
    fn format_report_shows_hostname_over_ip() {
        let state = sample_state();
        let report = format_report(&state);
        let lines: Vec<&str> = report.lines().collect();
        // Hop 1 has hostname "router"
        assert!(lines[3].contains("router"), "should show hostname");
        // Hop 2 has no hostname, should show IP
        assert!(lines[4].contains("10.0.0.1"), "should show IP when no hostname");
    }

    #[test]
    fn format_report_rtt_values_in_milliseconds() {
        let state = sample_state();
        let report = format_report(&state);
        let lines: Vec<&str> = report.lines().collect();
        // Hop 1: last=1.2ms, avg=1.1ms (1100us), best=0.8ms, worst=2.3ms
        let hop1 = lines[3];
        assert!(hop1.contains("1.2"), "last RTT should be 1.2");
        assert!(hop1.contains("1.1"), "avg RTT should be 1.1");
        assert!(hop1.contains("0.8"), "best RTT should be 0.8");
        assert!(hop1.contains("2.3"), "worst RTT should be 2.3");
    }
}
