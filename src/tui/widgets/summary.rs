use std::time::{Duration, Instant};

use ratatui::style::{Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::Paragraph;

use crate::trace::state::TraceState;

/// Format an elapsed duration as "HH:MM:SS".
pub fn format_elapsed(started: Instant) -> String {
    format_duration(started.elapsed())
}

/// Testable helper: format a `Duration` as "HH:MM:SS".
pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Build the one-line summary string shown in the header bar.
///
/// When `target_index` is Some, includes a "[1/3]" indicator for multi-target mode.
pub fn build_summary_text(
    state: &TraceState,
    paused: bool,
    target_index: Option<(usize, usize)>,
) -> String {
    let elapsed = format_elapsed(state.started_at);
    let pause_indicator = if paused { " PAUSED" } else { "" };
    let target_indicator = match target_index {
        Some((idx, total)) if total > 1 => format!("[{}/{}] ", idx + 1, total),
        _ => String::new(),
    };
    format!(
        "latensee - {}{} -- round {} -- {}{}",
        target_indicator, state.target, state.round, elapsed, pause_indicator
    )
}

/// Create a styled ratatui `Paragraph` for the summary header.
pub fn summary_widget(
    state: &TraceState,
    paused: bool,
    target_index: Option<(usize, usize)>,
) -> Paragraph<'_> {
    let text = build_summary_text(state, paused, target_index);
    Paragraph::new(Text::raw(text)).style(Style::default().add_modifier(Modifier::BOLD))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{TargetInfo, TraceState};
    use std::net::{IpAddr, Ipv4Addr};

    fn make_state() -> TraceState {
        let target = TargetInfo {
            hostname: "example.com".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        };
        let mut state = TraceState::new(target, 30);
        state.round = 42;
        state
    }

    #[test]
    fn format_duration_formats_correctly() {
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration(Duration::from_secs(61)), "00:01:01");
        assert_eq!(format_duration(Duration::from_secs(3661)), "01:01:01");
        assert_eq!(format_duration(Duration::from_secs(86399)), "23:59:59");
    }

    #[test]
    fn build_summary_text_contains_target() {
        let state = make_state();
        let text = build_summary_text(&state, false, None);
        assert!(
            text.contains("example.com (93.184.216.34)"),
            "expected target in summary, got: {text}"
        );
    }

    #[test]
    fn build_summary_text_contains_round() {
        let state = make_state();
        let text = build_summary_text(&state, false, None);
        assert!(
            text.contains("round 42"),
            "expected round number in summary, got: {text}"
        );
    }

    #[test]
    fn build_summary_text_shows_paused_when_paused() {
        let state = make_state();
        let text = build_summary_text(&state, true, None);
        assert!(
            text.contains("PAUSED"),
            "expected PAUSED in summary, got: {text}"
        );
    }

    #[test]
    fn build_summary_text_no_paused_when_running() {
        let state = make_state();
        let text = build_summary_text(&state, false, None);
        assert!(
            !text.contains("PAUSED"),
            "should not contain PAUSED when running, got: {text}"
        );
    }

    #[test]
    fn build_summary_text_shows_target_indicator_for_multiple() {
        let state = make_state();
        let text = build_summary_text(&state, false, Some((0, 3)));
        assert!(
            text.contains("[1/3]"),
            "expected [1/3] indicator, got: {text}"
        );
    }

    #[test]
    fn build_summary_text_no_indicator_for_single_target() {
        let state = make_state();
        let text = build_summary_text(&state, false, Some((0, 1)));
        assert!(
            !text.contains("[1/1]"),
            "should not show indicator for single target, got: {text}"
        );
    }
}
