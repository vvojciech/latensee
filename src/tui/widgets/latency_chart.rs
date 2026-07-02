use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::{Dataset, GraphType};

use crate::config::Thresholds;
use crate::trace::state::HopState;

/// Extract (x, y) chart points from hop samples.
/// X = sample index (0..n), Y = RTT in milliseconds.
/// Skips samples where rtt is None (packet loss).
pub fn build_latency_data(hop: &HopState) -> Vec<(f64, f64)> {
    hop.samples
        .iter()
        .enumerate()
        .filter_map(|(i, probe)| {
            probe
                .rtt
                .map(|rtt| (i as f64, rtt.as_secs_f64() * 1000.0))
        })
        .collect()
}

/// Extract loss points: (index, 0.0) for each sample where rtt is None.
pub fn build_loss_data(hop: &HopState) -> Vec<(f64, f64)> {
    hop.samples
        .iter()
        .enumerate()
        .filter_map(|(i, probe)| {
            if probe.rtt.is_none() {
                Some((i as f64, 0.0))
            } else {
                None
            }
        })
        .collect()
}

/// Compute Y-axis bounds with 10% padding on each side.
/// Returns (0.0, 1.0) for empty data.
pub fn compute_y_bounds(data: &[(f64, f64)]) -> (f64, f64) {
    if data.is_empty() {
        return (0.0, 1.0);
    }

    let min_y = data.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
    let max_y = data
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);

    let range = max_y - min_y;
    let padding = if range == 0.0 {
        0.1 * min_y.max(1.0)
    } else {
        0.1 * range
    };

    ((min_y - padding).max(0.0), max_y + padding)
}

fn point_zone(y: f64, thresholds: &Thresholds) -> u8 {
    if y >= thresholds.rtt_crit_ms {
        2
    } else if y >= thresholds.rtt_warn_ms {
        1
    } else {
        0
    }
}

fn zone_color(zone: u8) -> Color {
    match zone {
        0 => Color::Green,
        1 => Color::Yellow,
        _ => Color::Red,
    }
}

pub struct ColorRun {
    pub zone: u8,
    pub points: Vec<(f64, f64)>,
}

/// Interpolated points where the segment p0 -> p1 strictly crosses a
/// threshold, ordered by traversal direction from p0 to p1.
fn crossing_points(p0: (f64, f64), p1: (f64, f64), thresholds: &Thresholds) -> Vec<(f64, f64)> {
    let (x0, y0) = p0;
    let (x1, y1) = p1;
    let mut crossings: Vec<(f64, f64)> = [thresholds.rtt_warn_ms, thresholds.rtt_crit_ms]
        .into_iter()
        .filter(|&t| (y0 < t && y1 > t) || (y0 > t && y1 < t))
        .map(|t| (x0 + (x1 - x0) * (t - y0) / (y1 - y0), t))
        .collect();
    if y1 < y0 {
        crossings.reverse();
    }
    crossings
}

/// Split data into contiguous runs by threshold zone.
/// Runs split at interpolated threshold crossings so the color changes
/// exactly at the threshold height; crossing points are shared between
/// adjacent runs so lines connect.
pub fn build_color_runs(data: &[(f64, f64)], thresholds: &Thresholds) -> Vec<ColorRun> {
    if data.is_empty() {
        return Vec::new();
    }

    let mut runs: Vec<ColorRun> = Vec::new();
    let mut current_zone = point_zone(data[0].1, thresholds);
    let mut current_points = vec![data[0]];

    for pair in data.windows(2) {
        let (p0, p1) = (pair[0], pair[1]);
        let mut start = p0;
        for end in crossing_points(p0, p1, thresholds)
            .into_iter()
            .chain(std::iter::once(p1))
        {
            // classify by midpoint so a sub-segment merely touching a
            // threshold at one end keeps the zone it lies in
            let zone = point_zone((start.1 + end.1) / 2.0, thresholds);
            if zone != current_zone {
                runs.push(ColorRun {
                    zone: current_zone,
                    points: std::mem::take(&mut current_points),
                });
                current_points.push(start);
                current_zone = zone;
            }
            current_points.push(end);
            start = end;
        }
    }

    runs.push(ColorRun {
        zone: current_zone,
        points: current_points,
    });

    runs
}

/// Holds owned run data and loss markers so datasets can borrow from it.
pub struct ChartRunData {
    pub runs: Vec<ColorRun>,
    pub loss: Vec<(f64, f64)>,
}

/// Build chart datasets from pre-computed run data.
pub fn build_chart_datasets(run_data: &ChartRunData) -> Vec<Dataset<'_>> {
    let mut datasets: Vec<Dataset<'_>> = run_data
        .runs
        .iter()
        .map(|run| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(zone_color(run.zone)))
                .data(&run.points)
        })
        .collect();

    if !run_data.loss.is_empty() {
        datasets.push(
            Dataset::default()
                .name("loss")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&run_data.loss),
        );
    }

    datasets
}

/// Build the chart title line for a given hop.
pub fn latency_chart_title(hop: &HopState) -> String {
    let label = match (&hop.hostname, &hop.addr) {
        (Some(name), _) => name.clone(),
        (None, Some(addr)) => addr.to_string(),
        (None, None) => "???".to_string(),
    };
    format!("Latency (hop {}: {})", hop.ttl, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopStats, ProbeResult};
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    fn make_probe(rtt_us: Option<u64>) -> ProbeResult {
        ProbeResult {
            rtt: rtt_us.map(Duration::from_micros),
            addr: None,
            error: None,
        }
    }

    fn make_hop(ttl: u8, probes: Vec<ProbeResult>) -> HopState {
        HopState {
            ttl,
            addr: None,
            hostname: None,
            samples: VecDeque::from(probes),
            stats: HopStats::new(),
        }
    }

    // -- build_latency_data tests --

    #[test]
    fn build_latency_data_skips_none_rtt() {
        let hop = make_hop(
            1,
            vec![
                make_probe(Some(5_000)),
                make_probe(None),
                make_probe(Some(10_000)),
                make_probe(None),
                make_probe(Some(15_000)),
            ],
        );

        let data = build_latency_data(&hop);

        assert_eq!(data.len(), 3);
        assert_eq!(data[0].0, 0.0);
        assert_eq!(data[1].0, 2.0);
        assert_eq!(data[2].0, 4.0);
    }

    #[test]
    fn build_latency_data_empty_samples_returns_empty() {
        let hop = make_hop(1, vec![]);
        let data = build_latency_data(&hop);
        assert!(data.is_empty());
    }

    #[test]
    fn build_latency_data_converts_duration_to_ms() {
        let hop = make_hop(
            1,
            vec![
                make_probe(Some(1_500)),  // 1.5 ms
                make_probe(Some(25_000)), // 25.0 ms
                make_probe(Some(100)),    // 0.1 ms
            ],
        );

        let data = build_latency_data(&hop);

        assert_eq!(data.len(), 3);
        assert!((data[0].1 - 1.5).abs() < 0.001);
        assert!((data[1].1 - 25.0).abs() < 0.001);
        assert!((data[2].1 - 0.1).abs() < 0.001);
    }

    // -- build_loss_data tests --

    #[test]
    fn build_loss_data_captures_timeouts() {
        let hop = make_hop(
            1,
            vec![
                make_probe(Some(5_000)),
                make_probe(None),
                make_probe(Some(10_000)),
                make_probe(None),
            ],
        );
        let loss = build_loss_data(&hop);
        assert_eq!(loss.len(), 2);
        assert_eq!(loss[0], (1.0, 0.0));
        assert_eq!(loss[1], (3.0, 0.0));
    }

    #[test]
    fn build_loss_data_empty_when_no_loss() {
        let hop = make_hop(1, vec![make_probe(Some(5_000)), make_probe(Some(10_000))]);
        let loss = build_loss_data(&hop);
        assert!(loss.is_empty());
    }

    // -- compute_y_bounds tests --

    #[test]
    fn compute_y_bounds_with_data_returns_padded_range() {
        let data = vec![(0.0, 10.0), (1.0, 20.0), (2.0, 30.0)];
        let (min, max) = compute_y_bounds(&data);
        // range = 20, padding = 2
        assert!((min - 8.0).abs() < 0.001);
        assert!((max - 32.0).abs() < 0.001);
    }

    #[test]
    fn compute_y_bounds_empty_data_returns_default() {
        let (min, max) = compute_y_bounds(&[]);
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn compute_y_bounds_single_point_adds_padding() {
        let data = vec![(0.0, 50.0)];
        let (min, max) = compute_y_bounds(&data);
        assert!((min - 45.0).abs() < 0.001);
        assert!((max - 55.0).abs() < 0.001);
    }

    // -- color runs tests --

    fn defaults() -> Thresholds {
        Thresholds::default()
    }

    #[test]
    fn runs_empty_data() {
        let runs = build_color_runs(&[], &defaults());
        assert!(runs.is_empty());
    }

    #[test]
    fn runs_single_zone_one_run() {
        let data = vec![(0.0, 10.0), (1.0, 20.0), (2.0, 30.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].zone, 0);
        assert_eq!(runs[0].points.len(), 3);
    }

    fn assert_point_eq(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 0.001 && (actual.1 - expected.1).abs() < 0.001,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn runs_split_at_interpolated_threshold_crossing() {
        // green(10) -> yellow(80): warn=50 crossed at x = (50-10)/(80-10) = 4/7
        let data = vec![(0.0, 10.0), (1.0, 80.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].zone, 0);
        assert_eq!(runs[0].points.len(), 2);
        assert_point_eq(runs[0].points[1], (4.0 / 7.0, 50.0));
        assert_eq!(runs[1].zone, 1);
        assert_eq!(runs[1].points.len(), 2);
        assert_point_eq(runs[1].points[0], (4.0 / 7.0, 50.0));
        assert_point_eq(runs[1].points[1], (1.0, 80.0));
    }

    #[test]
    fn runs_multiple_transitions() {
        let data = vec![
            (0.0, 10.0),  // green
            (1.0, 80.0),  // yellow
            (2.0, 200.0), // red
            (3.0, 20.0),  // green
        ];
        let runs = build_color_runs(&data, &defaults());
        // the falling 200 -> 20 segment crosses crit then warn,
        // so it contributes a yellow slice on the way down
        let zones: Vec<u8> = runs.iter().map(|r| r.zone).collect();
        assert_eq!(zones, vec![0, 1, 2, 1, 0]);
        // rising warn crossing
        assert_point_eq(runs[0].points[1], (4.0 / 7.0, 50.0));
        // rising crit crossing: x = 1 + (150-80)/(200-80)
        assert_point_eq(runs[1].points[2], (1.0 + 70.0 / 120.0, 150.0));
        // falling crit crossing: x = 2 + (150-200)/(20-200)
        assert_point_eq(runs[2].points[2], (2.0 + 50.0 / 180.0, 150.0));
        // falling warn crossing: x = 2 + (50-200)/(20-200)
        assert_point_eq(runs[3].points[1], (2.0 + 150.0 / 180.0, 50.0));
        assert_point_eq(runs[4].points[1], (3.0, 20.0));
    }

    #[test]
    fn runs_double_crossing_in_single_segment() {
        // green(10) -> red(200): one segment crosses warn and crit
        let data = vec![(0.0, 10.0), (1.0, 200.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].zone, 0);
        assert_point_eq(runs[0].points[1], (40.0 / 190.0, 50.0));
        assert_eq!(runs[1].zone, 1);
        assert_point_eq(runs[1].points[0], (40.0 / 190.0, 50.0));
        assert_point_eq(runs[1].points[1], (140.0 / 190.0, 150.0));
        assert_eq!(runs[2].zone, 2);
        assert_point_eq(runs[2].points[0], (140.0 / 190.0, 150.0));
        assert_point_eq(runs[2].points[1], (1.0, 200.0));
    }

    #[test]
    fn runs_falling_crossing() {
        // yellow(80) -> green(10): warn crossed at x = (50-80)/(10-80) = 3/7
        let data = vec![(0.0, 80.0), (1.0, 10.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].zone, 1);
        assert_point_eq(runs[0].points[1], (3.0 / 7.0, 50.0));
        assert_eq!(runs[1].zone, 0);
        assert_point_eq(runs[1].points[0], (3.0 / 7.0, 50.0));
        assert_point_eq(runs[1].points[1], (1.0, 10.0));
    }

    #[test]
    fn runs_sample_exactly_on_threshold_rising_stays_green() {
        // the segment lies below the threshold, only touching it at the end
        let data = vec![(0.0, 10.0), (1.0, 50.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].zone, 0);
        assert_eq!(runs[0].points.len(), 2);
    }

    #[test]
    fn runs_sample_exactly_on_threshold_falling_splits_at_sample() {
        // first point sits on warn; the descending segment is green
        let data = vec![(0.0, 50.0), (1.0, 10.0)];
        let runs = build_color_runs(&data, &defaults());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].zone, 1);
        assert_eq!(runs[0].points, vec![(0.0, 50.0)]);
        assert_eq!(runs[1].zone, 0);
        assert_eq!(runs[1].points, vec![(0.0, 50.0), (1.0, 10.0)]);
    }

    #[test]
    fn chart_datasets_match_run_count_plus_loss() {
        let run_data = ChartRunData {
            runs: build_color_runs(
                &[(0.0, 10.0), (1.0, 80.0)],
                &defaults(),
            ),
            loss: vec![(2.0, 0.0)],
        };
        let ds = build_chart_datasets(&run_data);
        assert_eq!(ds.len(), 3); // 2 runs + 1 loss
    }

    #[test]
    fn chart_datasets_empty_when_no_data() {
        let run_data = ChartRunData {
            runs: vec![],
            loss: vec![],
        };
        let ds = build_chart_datasets(&run_data);
        assert!(ds.is_empty());
    }

    // -- latency_chart_title tests --

    #[test]
    fn latency_chart_title_with_hostname() {
        let mut hop = make_hop(3, vec![]);
        hop.hostname = Some("gateway.example.com".to_string());
        hop.addr = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));

        assert_eq!(
            latency_chart_title(&hop),
            "Latency (hop 3: gateway.example.com)"
        );
    }

    #[test]
    fn latency_chart_title_with_ip_only() {
        let mut hop = make_hop(5, vec![]);
        hop.addr = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

        assert_eq!(latency_chart_title(&hop), "Latency (hop 5: 10.0.0.1)");
    }

    #[test]
    fn latency_chart_title_with_neither_shows_unknown() {
        let hop = make_hop(2, vec![]);
        assert_eq!(latency_chart_title(&hop), "Latency (hop 2: ???)");
    }
}
