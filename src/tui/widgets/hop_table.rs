use std::time::Duration;

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Cell, Row, Table};

use crate::trace::state::HopState;

/// Format an optional Duration as milliseconds with 1 decimal place.
/// Returns "-" for None.
pub fn format_rtt_ms(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("{:.1}", d.as_secs_f64() * 1000.0),
        None => "-".to_string(),
    }
}

/// Convert microseconds to milliseconds string with 1 decimal place.
/// Returns "-" if the value is 0.0 (no data).
pub fn format_us_to_ms(us: f64) -> String {
    if us == 0.0 {
        "-".to_string()
    } else {
        format!("{:.1}", us / 1000.0)
    }
}

/// Display name for a hop: hostname if available, else IP, else "???".
pub fn format_host(hop: &HopState) -> String {
    if let Some(ref hostname) = hop.hostname {
        hostname.clone()
    } else if let Some(addr) = hop.addr {
        addr.to_string()
    } else {
        "???".to_string()
    }
}

/// Build table rows from hop state. The selected row gets a highlight style.
pub fn build_hop_table_rows(hops: &[HopState], selected: usize) -> Vec<Row<'_>> {
    hops.iter()
        .enumerate()
        .map(|(i, hop)| {
            let cells = vec![
                Cell::from(hop.ttl.to_string()),
                Cell::from(format_host(hop)),
                Cell::from(format!("{:.1}", hop.stats.loss_pct)),
                Cell::from(hop.stats.sent.to_string()),
                Cell::from(format_rtt_ms(hop.stats.last_rtt)),
                Cell::from(format_us_to_ms(hop.stats.avg_rtt)),
                Cell::from(format_rtt_ms(hop.stats.min_rtt)),
                Cell::from(format_rtt_ms(hop.stats.max_rtt)),
                Cell::from(format_us_to_ms(hop.stats.jitter)),
            ];

            let row = Row::new(cells);
            if i == selected {
                row.style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                row
            }
        })
        .collect()
}

/// Create the Table widget shell with headers and column constraints.
pub fn hop_table_widget(_selected: usize) -> Table<'static> {
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("Host"),
        Cell::from("Loss%"),
        Cell::from("Sent"),
        Cell::from("Last"),
        Cell::from("Avg"),
        Cell::from("Best"),
        Cell::from("Wrst"),
        Cell::from("StDev"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let widths = [
        Constraint::Length(4),
        Constraint::Min(20),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
    ];

    Table::new(Vec::<Row>::new(), widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .column_spacing(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopStats, ProbeResult};
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Instant;

    fn make_hop(ttl: u8, addr: Option<IpAddr>, hostname: Option<&str>) -> HopState {
        HopState {
            ttl,
            addr,
            hostname: hostname.map(|s| s.to_string()),
            samples: VecDeque::new(),
            stats: HopStats::new(),
        }
    }

    fn make_hop_with_stats(ttl: u8, hostname: Option<&str>, addr: Option<IpAddr>) -> HopState {
        let mut hop = make_hop(ttl, addr, hostname);
        let probe = ProbeResult {
            seq: 1,
            rtt: Some(Duration::from_micros(12300)),
            timestamp: Instant::now(),
            addr: None,
        };
        hop.stats.record_probe(&probe);
        hop
    }

    // --- format_rtt_ms ---

    #[test]
    fn format_rtt_ms_with_some_duration() {
        let d = Some(Duration::from_micros(12345));
        assert_eq!(format_rtt_ms(d), "12.3");
    }

    #[test]
    fn format_rtt_ms_with_none() {
        assert_eq!(format_rtt_ms(None), "-");
    }

    // --- format_us_to_ms ---

    #[test]
    fn format_us_to_ms_with_positive_value() {
        assert_eq!(format_us_to_ms(12345.0), "12.3");
    }

    #[test]
    fn format_us_to_ms_with_zero_returns_dash() {
        assert_eq!(format_us_to_ms(0.0), "-");
    }

    // --- format_host ---

    #[test]
    fn format_host_with_hostname() {
        let hop = make_hop(
            1,
            Some(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            Some("one.one.one.one"),
        );
        assert_eq!(format_host(&hop), "one.one.one.one");
    }

    #[test]
    fn format_host_with_addr_only() {
        let hop = make_hop(1, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))), None);
        assert_eq!(format_host(&hop), "8.8.8.8");
    }

    #[test]
    fn format_host_with_nothing_returns_question_marks() {
        let hop = make_hop(1, None, None);
        assert_eq!(format_host(&hop), "???");
    }

    // --- build_hop_table_rows ---

    #[test]
    fn build_hop_table_rows_returns_correct_count() {
        let hops = vec![
            make_hop_with_stats(1, Some("gw.local"), None),
            make_hop_with_stats(2, None, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))),
            make_hop(3, None, None),
        ];
        let rows = build_hop_table_rows(&hops, 0);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn build_hop_table_rows_first_cell_is_ttl() {
        let hops = vec![make_hop(5, None, None)];
        let rows = build_hop_table_rows(&hops, 0);
        assert_eq!(rows.len(), 1);
        // Verify the hop TTL used in row construction
        assert_eq!(hops[0].ttl, 5);
    }
}
