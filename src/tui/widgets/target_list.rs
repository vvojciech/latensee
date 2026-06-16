use parking_lot::RwLock;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Cell, Row, Table};
use std::sync::Arc;
use std::time::Instant;

use crate::report::format::{format_rtt_ms, format_us_to_ms};
use crate::trace::state::TraceState;

fn format_elapsed(started: Instant) -> String {
    let total_secs = started.elapsed().as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn destination_summary(state: &TraceState) -> String {
    let dest = state
        .hops
        .iter()
        .rev()
        .find(|h| h.stats.received > 0);

    match dest {
        Some(hop) => format!(
            "  last {}  avg {}  loss {:.1}%",
            format_rtt_ms(hop.stats.last_rtt),
            format_us_to_ms(hop.stats.avg_rtt),
            hop.stats.loss_pct,
        ),
        None => "  last -  avg -  loss -".to_string(),
    }
}

pub fn build_target_list_rows(
    states: &[Arc<RwLock<TraceState>>],
    active: usize,
    paused: bool,
) -> Vec<Row<'static>> {
    states
        .iter()
        .enumerate()
        .map(|(i, state)| {
            let state = state.read();
            let marker = if i == active { ">" } else { " " };
            let pause = if paused && i == active {
                "  PAUSED"
            } else {
                ""
            };
            let latency = destination_summary(&state);
            let text = format!(
                " {} {} ({})  round {}  {}{}{}",
                marker,
                state.target.hostname,
                state.target.addr,
                state.round,
                format_elapsed(state.started_at),
                latency,
                pause,
            );

            let row = Row::new(vec![Cell::from(text)]);
            if i == active {
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

pub fn target_list_widget() -> Table<'static> {
    let header = Row::new(vec![Cell::from("latensee")]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let widths = [Constraint::Min(40)];

    Table::new(Vec::<Row>::new(), widths)
        .header(header)
        .column_spacing(0)
        .row_highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopState, HopStats, ProbeResult, TargetInfo};
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    fn make_states(count: usize) -> Vec<Arc<RwLock<TraceState>>> {
        (0..count)
            .map(|i| {
                let target = TargetInfo {
                    hostname: format!("target-{}.example.com", i),
                    addr: IpAddr::V4(Ipv4Addr::new(10, 0, 0, i as u8 + 1)),
                };
                let mut state = TraceState::new(target, 30);
                state.round = (i as u64 + 1) * 10;
                Arc::new(RwLock::new(state))
            })
            .collect()
    }

    fn make_state_with_hops() -> Arc<RwLock<TraceState>> {
        let target = TargetInfo {
            hostname: "example.com".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        };
        let mut state = TraceState::new(target, 30);
        state.round = 10;

        let mut hop = HopState::new(1);
        hop.addr = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let probe = ProbeResult {
            rtt: Some(Duration::from_micros(12300)),
            addr: None,
            error: None,
        };
        hop.add_probe(probe, 300);
        state.hops.push(hop);

        Arc::new(RwLock::new(state))
    }

    #[test]
    fn target_list_rows_match_state_count() {
        let states = make_states(3);
        let rows = build_target_list_rows(&states, 0, false);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn single_target_shows_one_row() {
        let states = make_states(1);
        let rows = build_target_list_rows(&states, 0, false);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn target_list_widget_creates_table() {
        let _table = target_list_widget();
    }

    #[test]
    fn row_shows_dashes_when_no_hops() {
        let state = TraceState::new(
            TargetInfo {
                hostname: "test.com".to_string(),
                addr: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            },
            30,
        );
        let summary = destination_summary(&state);
        assert!(summary.contains("last -"), "no hops should show dash for last: {summary}");
        assert!(summary.contains("avg -"), "no hops should show dash for avg: {summary}");
        assert!(summary.contains("loss -"), "no hops should show dash for loss: {summary}");
    }

    #[test]
    fn row_shows_latency_for_destination_hop() {
        let arc = make_state_with_hops();
        let state = arc.read();
        let summary = destination_summary(&state);
        assert!(summary.contains("last"), "should have last RTT: {summary}");
        assert!(summary.contains("avg"), "should have avg RTT: {summary}");
        assert!(summary.contains("loss 0.0%"), "should have loss pct: {summary}");
        assert!(!summary.contains("last -"), "should not show dashes when hops exist: {summary}");
    }
}
