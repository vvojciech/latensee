pub mod widgets;

use std::io::Stdout;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Axis, Block, Borders, Chart, Clear};
use ratatui::Frame;
use ratatui::Terminal;
use tokio_util::sync::CancellationToken;

use crate::trace::state::TraceState;
use widgets::help::help_widget;
use widgets::hop_table::{build_hop_table_rows, hop_table_widget};
use widgets::latency_chart::{build_chart_dataset, build_latency_data, compute_y_bounds, latency_chart_title};
use widgets::summary::summary_widget;

/// TUI application state, separate from trace data.
pub struct AppState {
    pub selected_hop: usize,
    pub paused: bool,
    pub show_help: bool,
    pub should_quit: bool,
    pub active_target: usize,
    pub target_count: usize,
}

impl AppState {
    pub fn new(target_count: usize) -> Self {
        Self {
            selected_hop: 0,
            paused: false,
            show_help: false,
            should_quit: false,
            active_target: 0,
            target_count: target_count.max(1),
        }
    }

    pub fn next_target(&mut self) {
        self.active_target = (self.active_target + 1) % self.target_count;
        self.selected_hop = 0;
    }

    pub fn prev_target(&mut self) {
        self.active_target = if self.active_target == 0 {
            self.target_count - 1
        } else {
            self.active_target - 1
        };
        self.selected_hop = 0;
    }

    pub fn next_hop(&mut self, max: usize) {
        if max == 0 {
            return;
        }
        if self.selected_hop < max.saturating_sub(1) {
            self.selected_hop += 1;
        }
    }

    pub fn prev_hop(&mut self) {
        self.selected_hop = self.selected_hop.saturating_sub(1);
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }
}

/// Prepare the terminal for TUI rendering.
pub fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, anyhow::Error> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
pub fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), anyhow::Error> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Dispatch a key event to the appropriate app state mutation.
pub fn handle_key_event(key: KeyEvent, app: &mut AppState, max_hops: usize) {
    if key.kind != KeyEventKind::Press {
        return;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.prev_hop(),
        KeyCode::Down | KeyCode::Char('j') => app.next_hop(max_hops),
        KeyCode::Char('p') => app.toggle_pause(),
        KeyCode::Char('h') | KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Tab => app.next_target(),
        KeyCode::BackTab => app.prev_target(),
        KeyCode::Char('r') => {} // reserved: reset stats
        KeyCode::Char('g') => {} // reserved: graph toggle
        _ => {}
    }
}

/// Minimum terminal height to show the latency chart. Below this, table-only mode.
const MIN_HEIGHT_FOR_CHART: u16 = 20;

/// Compute a centered rectangle within `area` at the given percentage size.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Render the full TUI frame: summary bar, hop table, latency chart, and optional help overlay.
pub fn render_frame(frame: &mut Frame, state: &TraceState, app: &AppState) {
    let area = frame.area();
    let show_chart = area.height >= MIN_HEIGHT_FOR_CHART;

    let chunks = if show_chart {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Percentage(50),
                Constraint::Min(5),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
            ])
            .split(area)
    };

    // Summary bar
    let target_index = Some((app.active_target, app.target_count));
    frame.render_widget(summary_widget(state, app.paused, target_index), chunks[0]);

    // Hop table with rows
    let rows = build_hop_table_rows(&state.hops, app.selected_hop);
    let table = hop_table_widget(app.selected_hop).rows(rows);
    frame.render_widget(table, chunks[1]);

    // Latency chart (only when tall enough and hops exist)
    if show_chart {
        let chart_chunk = chunks[2];
        if !state.hops.is_empty() && app.selected_hop < state.hops.len() {
            let hop = &state.hops[app.selected_hop];
            let data = build_latency_data(hop);
            let (y_min, y_max) = compute_y_bounds(&data);
            let x_max = if data.is_empty() { 1.0 } else { data.len() as f64 };
            let title = latency_chart_title(hop);

            let dataset = build_chart_dataset(&data);
            let chart = Chart::new(vec![dataset])
                .block(Block::default().borders(Borders::ALL).title(title))
                .x_axis(
                    Axis::default()
                        .bounds([0.0, x_max]),
                )
                .y_axis(
                    Axis::default()
                        .bounds([y_min, y_max])
                        .labels::<Vec<ratatui::text::Line>>(vec![
                            format!("{:.0}ms", y_min).into(),
                            format!("{:.0}ms", y_max).into(),
                        ]),
                );
            frame.render_widget(chart, chart_chunk);
        }
    }

    // Help overlay
    if app.show_help {
        let help_area = centered_rect(60, 50, area);
        frame.render_widget(Clear, help_area);
        frame.render_widget(help_widget(), help_area);
    }
}

const TICK_RATE: Duration = Duration::from_millis(67); // ~15fps

/// Main TUI event loop.
pub async fn run_tui(
    states: Vec<Arc<RwLock<TraceState>>>,
    cancel: CancellationToken,
) -> Result<(), anyhow::Error> {
    let mut terminal = setup_terminal()?;
    let mut app = AppState::new(states.len());

    let result = run_event_loop(&mut terminal, &mut app, &states, &cancel).await;

    restore_terminal(&mut terminal)?;
    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
    states: &[Arc<RwLock<TraceState>>],
    cancel: &CancellationToken,
) -> Result<(), anyhow::Error> {
    let mut tick_interval = tokio::time::interval(TICK_RATE);

    loop {
        // Render the active target
        let state = &states[app.active_target];
        let trace_state = state.read();
        let hop_count = trace_state.hop_count();

        terminal.draw(|frame| {
            render_frame(frame, &trace_state, app);
        })?;
        drop(trace_state);

        // Wait for next tick or event
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            _ = tick_interval.tick() => {
                // Poll for crossterm events (non-blocking)
                while event::poll(Duration::ZERO)? {
                    if let Event::Key(key) = event::read()? {
                        handle_key_event(key, app, hop_count);
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::state::{HopState, ProbeResult, TargetInfo};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use std::net::{IpAddr, Ipv4Addr};


    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn app_state_defaults() {
        let app = AppState::new(1);
        assert_eq!(app.selected_hop, 0);
        assert!(!app.paused);
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }

    #[test]
    fn next_hop_increments_within_bounds() {
        let mut app = AppState::new(1);
        app.next_hop(5);
        assert_eq!(app.selected_hop, 1);
        app.next_hop(5);
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn next_hop_clamps_at_max() {
        let mut app = AppState::new(1);
        app.selected_hop = 4;
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4, "should not exceed max - 1");
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4);
    }

    #[test]
    fn next_hop_zero_max_is_noop() {
        let mut app = AppState::new(1);
        app.next_hop(0);
        assert_eq!(app.selected_hop, 0);
    }

    #[test]
    fn prev_hop_decrements() {
        let mut app = AppState::new(1);
        app.selected_hop = 3;
        app.prev_hop();
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn prev_hop_clamps_at_zero() {
        let mut app = AppState::new(1);
        app.prev_hop();
        assert_eq!(app.selected_hop, 0, "should not go below 0");
    }

    #[test]
    fn toggle_pause_flips() {
        let mut app = AppState::new(1);
        assert!(!app.paused);
        app.toggle_pause();
        assert!(app.paused);
        app.toggle_pause();
        assert!(!app.paused);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = AppState::new(1);
        assert!(!app.show_help);
        app.toggle_help();
        assert!(app.show_help);
        app.toggle_help();
        assert!(!app.show_help);
    }

    #[test]
    fn key_q_sets_should_quit() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Char('q')), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn key_esc_sets_should_quit() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Esc), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn key_down_increments_hop() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_j_increments_hop() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Char('j')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_up_decrements_hop() {
        let mut app = AppState::new(1);
        app.selected_hop = 2;
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_k_decrements_hop() {
        let mut app = AppState::new(1);
        app.selected_hop = 2;
        handle_key_event(press(KeyCode::Char('k')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_p_toggles_pause() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(app.paused);
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(!app.paused);
    }

    #[test]
    fn key_h_toggles_help() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Char('h')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn key_question_mark_toggles_help() {
        let mut app = AppState::new(1);
        handle_key_event(press(KeyCode::Char('?')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = AppState::new(1);
        let release = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        handle_key_event(release, &mut app, 5);
        assert!(!app.should_quit, "release events should be ignored");
    }

    // --- centered_rect tests ---

    #[test]
    fn centered_rect_is_smaller_than_area() {
        let area = Rect::new(0, 0, 100, 50);
        let inner = centered_rect(60, 40, area);
        assert!(inner.width < area.width, "inner width should be smaller");
        assert!(inner.height < area.height, "inner height should be smaller");
        assert!(inner.width > 0);
        assert!(inner.height > 0);
    }

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect::new(0, 0, 100, 50);
        let inner = centered_rect(60, 40, area);

        let left_margin = inner.x;
        let right_margin = area.width - (inner.x + inner.width);
        // Margins should be roughly equal (within 1 due to rounding)
        assert!(
            left_margin.abs_diff(right_margin) <= 1,
            "horizontal margins should be roughly equal: left={left_margin}, right={right_margin}"
        );

        let top_margin = inner.y;
        let bottom_margin = area.height - (inner.y + inner.height);
        assert!(
            top_margin.abs_diff(bottom_margin) <= 1,
            "vertical margins should be roughly equal: top={top_margin}, bottom={bottom_margin}"
        );
    }

    // --- render_frame tests ---

    fn make_trace_state() -> TraceState {
        let target = TargetInfo {
            hostname: "example.com".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
        };
        TraceState::new(target, 30)
    }

    fn make_hop_with_samples(ttl: u8) -> HopState {
        let mut hop = HopState::new(ttl);
        hop.addr = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, ttl)));
        hop.hostname = Some(format!("hop-{}.example.com", ttl));
        for seq in 0..5u64 {
            let probe = ProbeResult {
                rtt: Some(Duration::from_micros(1000 + seq * 500)),
                addr: None,
            };
            hop.add_probe(probe, 50);
        }
        hop
    }

    #[test]
    fn render_frame_empty_state_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_trace_state();
        let app = AppState::new(1);

        terminal
            .draw(|frame| {
                render_frame(frame, &state, &app);
            })
            .unwrap();
    }

    #[test]
    fn render_frame_with_hops_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_trace_state();
        state.hops.push(make_hop_with_samples(1));
        state.hops.push(make_hop_with_samples(2));
        state.hops.push(make_hop_with_samples(3));
        let mut app = AppState::new(1);
        app.selected_hop = 1;

        terminal
            .draw(|frame| {
                render_frame(frame, &state, &app);
            })
            .unwrap();
    }

    #[test]
    fn render_frame_with_help_overlay_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_trace_state();
        let mut app = AppState::new(1);
        app.show_help = true;

        terminal
            .draw(|frame| {
                render_frame(frame, &state, &app);
            })
            .unwrap();
    }

    #[test]
    fn render_frame_short_terminal_skips_chart() {
        let backend = TestBackend::new(80, 15); // below MIN_HEIGHT_FOR_CHART
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_trace_state();
        state.hops.push(make_hop_with_samples(1));
        let app = AppState::new(1);

        // Should not panic even with small terminal
        terminal
            .draw(|frame| {
                render_frame(frame, &state, &app);
            })
            .unwrap();
    }

    // --- target cycling tests ---

    #[test]
    fn tab_cycles_active_target_forward() {
        let mut app = AppState::new(3);
        assert_eq!(app.active_target, 0);
        handle_key_event(press(KeyCode::Tab), &mut app, 5);
        assert_eq!(app.active_target, 1);
        handle_key_event(press(KeyCode::Tab), &mut app, 5);
        assert_eq!(app.active_target, 2);
    }

    #[test]
    fn shift_tab_cycles_active_target_backward() {
        let mut app = AppState::new(3);
        app.active_target = 2;
        let shift_tab = KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        handle_key_event(shift_tab, &mut app, 5);
        assert_eq!(app.active_target, 1);
        handle_key_event(shift_tab, &mut app, 5);
        assert_eq!(app.active_target, 0);
    }

    #[test]
    fn active_target_wraps_forward() {
        let mut app = AppState::new(3);
        app.active_target = 2;
        app.next_target();
        assert_eq!(app.active_target, 0, "should wrap from last to first");
    }

    #[test]
    fn active_target_wraps_backward() {
        let mut app = AppState::new(3);
        app.prev_target();
        assert_eq!(app.active_target, 2, "should wrap from first to last");
    }

    #[test]
    fn target_switch_resets_selected_hop() {
        let mut app = AppState::new(3);
        app.selected_hop = 5;
        app.next_target();
        assert_eq!(app.selected_hop, 0, "switching target should reset hop selection");
    }

    #[test]
    fn single_target_tab_stays_at_zero() {
        let mut app = AppState::new(1);
        app.next_target();
        assert_eq!(app.active_target, 0);
        app.prev_target();
        assert_eq!(app.active_target, 0);
    }
}
