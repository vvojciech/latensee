use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Cell, Row, Table};

use crate::config::Thresholds;
use crate::report::format::{format_host, format_rtt_ms, format_us_to_ms};
use crate::trace::state::HopState;
use crate::tui::health::{health_fg, loss_health, rtt_health};

/// Build table rows from hop state. The selected row gets a highlight style.
pub fn build_hop_table_rows<'a>(
    hops: &'a [HopState],
    selected: usize,
    thresholds: &'a Thresholds,
) -> Vec<Row<'a>> {
    hops.iter()
        .enumerate()
        .map(|(i, hop)| {
            let loss_color = health_fg(loss_health(
                hop.stats.loss_pct,
                hop.stats.received,
                thresholds,
            ));
            let rtt_ms = hop.stats.last_rtt.map(|d| d.as_secs_f64() * 1000.0);
            let rtt_color = health_fg(rtt_health(rtt_ms, thresholds));

            let cells = vec![
                Cell::from(hop.ttl.to_string()),
                Cell::from(format_host(hop)),
                Cell::from(format!("{:.1}", hop.stats.loss_pct))
                    .style(Style::default().fg(loss_color)),
                Cell::from(hop.stats.sent.to_string()),
                Cell::from(if hop.stats.errors > 0 {
                    hop.stats.errors.to_string()
                } else {
                    "-".into()
                }),
                Cell::from(format_rtt_ms(hop.stats.last_rtt))
                    .style(Style::default().fg(rtt_color)),
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
pub fn hop_table_widget() -> Table<'static> {
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("Host"),
        Cell::from("Loss%"),
        Cell::from("Sent"),
        Cell::from("Errs"),
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
        Constraint::Length(5),
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
    use std::time::Duration;


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
            rtt: Some(Duration::from_micros(12300)),
            addr: None,
            error: None,
        };
        hop.stats.record_probe(&probe);
        hop
    }

    // --- build_hop_table_rows ---

    #[test]
    fn build_hop_table_rows_returns_correct_count() {
        let hops = vec![
            make_hop_with_stats(1, Some("gw.local"), None),
            make_hop_with_stats(2, None, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))),
            make_hop(3, None, None),
        ];
        let t = Thresholds::default();
        let rows = build_hop_table_rows(&hops, 0, &t);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn build_hop_table_rows_first_cell_is_ttl() {
        let hops = vec![make_hop(5, None, None)];
        let t = Thresholds::default();
        let rows = build_hop_table_rows(&hops, 0, &t);
        assert_eq!(rows.len(), 1);
        assert_eq!(hops[0].ttl, 5);
    }
}
