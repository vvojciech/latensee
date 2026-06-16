use parking_lot::RwLock;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Cell, Row, Table};
use std::sync::Arc;
use std::time::Instant;

use crate::trace::state::TraceState;

fn format_elapsed(started: Instant) -> String {
    let total_secs = started.elapsed().as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
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
            let text = format!(
                " {} {} ({})  round {}  {}{}",
                marker,
                state.target.hostname,
                state.target.addr,
                state.round,
                format_elapsed(state.started_at),
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
    use crate::trace::state::TargetInfo;
    use std::net::{IpAddr, Ipv4Addr};

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
}
