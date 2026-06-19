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

/// Holds the split data needed for multi-color chart rendering.
/// All vecs own their data so they outlive the Dataset references.
pub struct ChartData {
    pub good: Vec<(f64, f64)>,
    pub warn: Vec<(f64, f64)>,
    pub crit: Vec<(f64, f64)>,
    pub loss: Vec<(f64, f64)>,
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

/// Split raw data into color zones with boundary point duplication.
/// When consecutive points cross a threshold, both zones get the transition
/// point so the lines connect seamlessly at the color change.
pub fn prepare_chart_data(
    data: &[(f64, f64)],
    hop: &HopState,
    thresholds: &Thresholds,
) -> ChartData {
    let mut good = Vec::new();
    let mut warn = Vec::new();
    let mut crit = Vec::new();

    let mut prev_zone: Option<u8> = None;

    for (i, &point) in data.iter().enumerate() {
        let zone = point_zone(point.1, thresholds);

        if let Some(pz) = prev_zone {
            if zone != pz {
                let prev_point = data[i - 1];
                // End cap: add current point to previous zone's dataset
                match pz {
                    0 => good.push(point),
                    1 => warn.push(point),
                    _ => crit.push(point),
                }
                // Start cap: add previous point to current zone's dataset
                match zone {
                    0 => good.push(prev_point),
                    1 => warn.push(prev_point),
                    _ => crit.push(prev_point),
                }
            }
        }

        match zone {
            0 => good.push(point),
            1 => warn.push(point),
            _ => crit.push(point),
        }

        prev_zone = Some(zone);
    }

    ChartData {
        good,
        warn,
        crit,
        loss: build_loss_data(hop),
    }
}

/// Build datasets that borrow from a ChartData struct.
pub fn build_chart_datasets(cd: &ChartData) -> Vec<Dataset<'_>> {
    let mut datasets = Vec::new();

    if !cd.good.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Green))
                .data(&cd.good),
        );
    }
    if !cd.warn.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Yellow))
                .data(&cd.warn),
        );
    }
    if !cd.crit.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&cd.crit),
        );
    }

    if !cd.loss.is_empty() {
        datasets.push(
            Dataset::default()
                .name("loss")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&cd.loss),
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

    // -- build_chart_datasets tests --

    fn defaults() -> Thresholds {
        Thresholds::default()
    }

    #[test]
    fn build_chart_datasets_empty_data_returns_empty() {
        let data: Vec<(f64, f64)> = vec![];
        let hop = make_hop(1, vec![]);
        let cd = prepare_chart_data(&data, &hop, &defaults());
        let ds = build_chart_datasets(&cd);
        assert!(ds.is_empty());
    }

    #[test]
    fn zone_split_duplicates_boundary_points() {
        // green -> yellow -> red: each transition duplicates the boundary
        let data = vec![
            (0.0, 10.0),   // green
            (1.0, 80.0),   // yellow
            (2.0, 200.0),  // red
        ];
        let hop = make_hop(1, vec![
            make_probe(Some(10_000)),
            make_probe(Some(80_000)),
            make_probe(Some(200_000)),
        ]);
        let cd = prepare_chart_data(&data, &hop, &defaults());
        // green: (0,10) + end cap (1,80) = 2
        assert_eq!(cd.good.len(), 2, "green should have original + end cap");
        // yellow: start cap (0,10) + (1,80) + end cap (2,200) = 3
        assert_eq!(cd.warn.len(), 3, "yellow should have start cap + original + end cap");
        // red: start cap (1,80) + (2,200) = 2
        assert_eq!(cd.crit.len(), 2, "red should have start cap + original");
    }

    #[test]
    fn zone_split_single_zone_no_duplicates() {
        let data = vec![(0.0, 10.0), (1.0, 20.0), (2.0, 30.0)];
        let hop = make_hop(1, vec![
            make_probe(Some(10_000)),
            make_probe(Some(20_000)),
            make_probe(Some(30_000)),
        ]);
        let cd = prepare_chart_data(&data, &hop, &defaults());
        assert_eq!(cd.good.len(), 3);
        assert!(cd.warn.is_empty());
        assert!(cd.crit.is_empty());
    }

    #[test]
    fn zone_split_alternating_zones() {
        // green -> yellow -> green
        let data = vec![
            (0.0, 10.0),  // green
            (1.0, 80.0),  // yellow
            (2.0, 10.0),  // green
        ];
        let hop = make_hop(1, vec![
            make_probe(Some(10_000)),
            make_probe(Some(80_000)),
            make_probe(Some(10_000)),
        ]);
        let cd = prepare_chart_data(&data, &hop, &defaults());
        // green: (0,10) + end cap (1,80) | start cap (1,80) + (2,10) = 4
        assert_eq!(cd.good.len(), 4, "green: 2 originals + 2 caps");
        // yellow: start cap (0,10) + (1,80) + end cap (2,10) = 3
        assert_eq!(cd.warn.len(), 3, "yellow: 1 original + 2 caps");
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
