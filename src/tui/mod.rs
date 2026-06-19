pub mod health;
pub mod widgets;

use std::io::Stdout;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
use widgets::latency_chart::{build_chart_datasets, build_color_runs, build_latency_data, build_loss_data, compute_y_bounds, latency_chart_title, ChartRunData};
use widgets::target_list::{build_target_list_rows, target_list_widget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    AddTarget,
}

/// TUI application state, separate from trace data.
pub struct AppState {
    pub selected_hop: usize,
    pub paused: bool,
    pub show_help: bool,
    pub show_chart: bool,
    pub should_quit: bool,
    pub reset_requested: bool,
    pub remove_requested: bool,
    pub active_target: usize,
    pub target_count: usize,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub pending_target: Option<String>,
    pub status_message: Option<String>,
    shared_paused: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(target_count: usize, shared_paused: Arc<AtomicBool>) -> Self {
        Self {
            selected_hop: 0,
            paused: false,
            show_help: false,
            show_chart: true,
            should_quit: false,
            reset_requested: false,
            remove_requested: false,
            active_target: 0,
            target_count: target_count.max(1),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            pending_target: None,
            status_message: None,
            shared_paused,
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
        self.shared_paused.store(self.paused, Ordering::Relaxed);
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn toggle_chart(&mut self) {
        self.show_chart = !self.show_chart;
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

    match app.input_mode {
        InputMode::AddTarget => handle_input_mode_key(key, app),
        InputMode::Normal => handle_normal_mode_key(key, app, max_hops),
    }
}

fn handle_normal_mode_key(key: KeyEvent, app: &mut AppState, max_hops: usize) {
    app.status_message = None;
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up => app.prev_target(),
        KeyCode::Down => app.next_target(),
        KeyCode::Char('k') => app.prev_hop(),
        KeyCode::Char('j') => app.next_hop(max_hops),
        KeyCode::Char('p') => app.toggle_pause(),
        KeyCode::Char('h') | KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Char('r') => app.reset_requested = true,
        KeyCode::Char('g') => app.toggle_chart(),
        KeyCode::Char('a') => {
            app.input_mode = InputMode::AddTarget;
            app.input_buffer.clear();
        }
        KeyCode::Char('d') | KeyCode::Char('x') => {
            if app.target_count > 1 {
                app.remove_requested = true;
            }
        }
        _ => {}
    }
}

fn handle_input_mode_key(key: KeyEvent, app: &mut AppState) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buffer.clear();
        }
        KeyCode::Enter => {
            if !app.input_buffer.is_empty() {
                app.pending_target = Some(app.input_buffer.clone());
            }
            app.input_mode = InputMode::Normal;
            app.input_buffer.clear();
        }
        KeyCode::Backspace => {
            app.input_buffer.pop();
        }
        KeyCode::Char(c) => {
            app.input_buffer.push(c);
        }
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

/// Render the full TUI frame: target list, hop table, latency chart, and optional help/input overlays.
pub fn render_frame(
    frame: &mut Frame,
    states: &[Arc<RwLock<TraceState>>],
    active_state: &TraceState,
    app: &AppState,
    thresholds: &crate::config::Thresholds,
) {
    let area = frame.area();
    let show_chart = app.show_chart && area.height >= MIN_HEIGHT_FOR_CHART;
    let show_input = app.input_mode != InputMode::Normal || app.status_message.is_some();
    let target_list_height = (states.len() as u16).max(1) + 1; // +1 for header

    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(target_list_height),
        Constraint::Length(1), // gap between target list and hop table
        if show_chart {
            Constraint::Percentage(50)
        } else {
            Constraint::Min(5)
        },
    ];
    if show_chart {
        constraints.push(Constraint::Min(5));
    }
    if show_input {
        constraints.push(Constraint::Length(1));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Target list
    let target_rows = build_target_list_rows(states, app.active_target, app.paused, thresholds);
    let target_table = target_list_widget().rows(target_rows);
    frame.render_widget(target_table, chunks[0]);

    // chunks[1] is the gap -- intentionally empty

    // Hop table with rows
    let rows = build_hop_table_rows(&active_state.hops, app.selected_hop, thresholds);
    let table = hop_table_widget().rows(rows);
    frame.render_widget(table, chunks[2]);

    // Latency chart (only when tall enough and hops exist)
    if show_chart {
        let chart_chunk = chunks[3];
        if !active_state.hops.is_empty() && app.selected_hop < active_state.hops.len() {
            let hop = &active_state.hops[app.selected_hop];
            let data = build_latency_data(hop);
            let (y_min, y_max) = compute_y_bounds(&data);
            let x_max = if data.is_empty() { 1.0 } else { data.len() as f64 };
            let title = latency_chart_title(hop);

            let run_data = ChartRunData {
                runs: build_color_runs(&data, thresholds),
                loss: build_loss_data(hop),
            };
            let datasets = build_chart_datasets(&run_data);
            let chart = Chart::new(datasets)
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

    // Input bar / status message
    if show_input {
        let input_chunk = chunks[chunks.len() - 1];
        let text = match app.input_mode {
            InputMode::AddTarget => format!("Add target: {}█", app.input_buffer),
            InputMode::Normal => app.status_message.clone().unwrap_or_default(),
        };
        let style = if app.status_message.is_some() && app.input_mode == InputMode::Normal {
            ratatui::style::Style::default().fg(ratatui::style::Color::Yellow)
        } else {
            ratatui::style::Style::default().fg(ratatui::style::Color::Cyan)
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(text).style(style),
            input_chunk,
        );
    }

    // Help overlay
    if app.show_help {
        let help_area = centered_rect(60, 50, area);
        frame.render_widget(Clear, help_area);
        frame.render_widget(help_widget(), help_area);
    }
}

const TICK_RATE: Duration = Duration::from_millis(67); // ~15fps

/// Configuration subset needed for spawning new trace engines at runtime.
#[derive(Clone)]
pub struct EngineConfig {
    pub protocol: crate::config::ProbeProtocol,
    pub timeout: f64,
    pub size: u16,
    pub port: u16,
    pub interval: f64,
    pub max_hops: u8,
    pub count: Option<u64>,
    pub no_dns: bool,
    pub ip_version: crate::config::IpVersion,
    pub thresholds: crate::config::Thresholds,
}

impl EngineConfig {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            protocol: config.protocol,
            timeout: config.timeout,
            size: config.size,
            port: config.port,
            interval: config.interval,
            max_hops: config.max_hops,
            count: config.count,
            no_dns: config.no_dns,
            ip_version: config.ip_version,
            thresholds: config.thresholds,
        }
    }
}

/// Main TUI event loop.
pub async fn run_tui(
    states: Vec<Arc<RwLock<TraceState>>>,
    target_cancels: Vec<CancellationToken>,
    cancel: CancellationToken,
    paused: Arc<AtomicBool>,
    engine_config: EngineConfig,
) -> Result<(), anyhow::Error> {
    let mut terminal = setup_terminal()?;
    let mut app = AppState::new(states.len(), paused);

    let result = run_event_loop(
        &mut terminal,
        &mut app,
        states,
        target_cancels,
        &cancel,
        &engine_config,
    )
    .await;

    restore_terminal(&mut terminal)?;
    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
    mut states: Vec<Arc<RwLock<TraceState>>>,
    mut target_cancels: Vec<CancellationToken>,
    cancel: &CancellationToken,
    engine_config: &EngineConfig,
) -> Result<(), anyhow::Error> {
    let mut tick_interval = tokio::time::interval(TICK_RATE);
    let mut last_round: u64 = 0;
    let mut needs_redraw = true;

    loop {
        let state = &states[app.active_target];
        let trace_state = state.read();
        let hop_count = trace_state.hop_count();
        let current_round = trace_state.round;

        if needs_redraw || current_round != last_round {
            terminal.draw(|frame| {
                render_frame(frame, &states, &trace_state, app, &engine_config.thresholds);
            })?;
            last_round = current_round;
            needs_redraw = false;
        }
        drop(trace_state);

        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            _ = tick_interval.tick() => {
                while event::poll(Duration::ZERO)? {
                    if let Event::Key(key) = event::read()? {
                        handle_key_event(key, app, hop_count);
                        needs_redraw = true;
                    }
                }
            }
        }

        if app.reset_requested {
            let state = &states[app.active_target];
            state.write().reset_all();
            app.reset_requested = false;
            last_round = 0;
            needs_redraw = true;
        }

        if let Some(target_str) = app.pending_target.take() {
            match spawn_target(
                &target_str,
                engine_config,
                cancel,
                &app.shared_paused,
                &mut states,
                &mut target_cancels,
            )
            .await
            {
                Ok(()) => {
                    app.target_count = states.len();
                    app.active_target = states.len() - 1;
                    app.selected_hop = 0;
                    last_round = 0;
                    app.status_message = None;
                }
                Err(e) => {
                    app.status_message = Some(format!("Failed to add target: {e}"));
                }
            }
            needs_redraw = true;
        }

        if app.remove_requested {
            app.remove_requested = false;
            if states.len() > 1 {
                let idx = app.active_target;
                target_cancels[idx].cancel();
                states.remove(idx);
                target_cancels.remove(idx);
                app.target_count = states.len();
                if app.active_target >= states.len() {
                    app.active_target = states.len() - 1;
                }
                app.selected_hop = 0;
                last_round = 0;
                needs_redraw = true;
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn spawn_target(
    target_str: &str,
    engine_config: &EngineConfig,
    cancel: &CancellationToken,
    paused: &Arc<AtomicBool>,
    states: &mut Vec<Arc<RwLock<TraceState>>>,
    target_cancels: &mut Vec<CancellationToken>,
) -> Result<(), anyhow::Error> {
    let addr = crate::config::resolve_target(target_str, &engine_config.ip_version).await?;
    let target_info = crate::trace::state::TargetInfo {
        hostname: target_str.to_string(),
        addr,
    };
    let state = Arc::new(RwLock::new(crate::trace::state::TraceState::new(
        target_info,
        engine_config.max_hops,
    )));
    states.push(Arc::clone(&state));

    let config = crate::config::Config {
        targets: vec![target_str.to_string()],
        interval: engine_config.interval,
        max_hops: engine_config.max_hops,
        count: engine_config.count,
        size: engine_config.size,
        timeout: engine_config.timeout,
        protocol: engine_config.protocol,
        port: engine_config.port,
        report: false,
        csv: false,
        json: false,
        no_dns: engine_config.no_dns,
        ip_version: engine_config.ip_version,
        thresholds: engine_config.thresholds,
    };

    let target_cancel = cancel.child_token();
    target_cancels.push(target_cancel.clone());

    let engine = crate::trace::TraceEngine::new(
        Arc::clone(&state),
        &config,
        Arc::clone(paused),
    );
    let engine_cancel = target_cancel.clone();
    tokio::spawn(async move {
        engine.run(engine_cancel).await;
    });

    let dns_state = Arc::clone(&state);
    let no_dns = engine_config.no_dns;
    let dns_cancel = target_cancel;
    tokio::spawn(async move {
        if let Ok(resolver) = crate::trace::dns::DnsResolver::new().await {
            crate::trace::dns::run_dns_resolver(dns_state, resolver, no_dns, dns_cancel).await;
        }
    });

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
        let app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert_eq!(app.selected_hop, 0);
        assert!(!app.paused);
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }

    #[test]
    fn next_hop_increments_within_bounds() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.next_hop(5);
        assert_eq!(app.selected_hop, 1);
        app.next_hop(5);
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn next_hop_clamps_at_max() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.selected_hop = 4;
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4, "should not exceed max - 1");
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4);
    }

    #[test]
    fn next_hop_zero_max_is_noop() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.next_hop(0);
        assert_eq!(app.selected_hop, 0);
    }

    #[test]
    fn prev_hop_decrements() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.selected_hop = 3;
        app.prev_hop();
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn prev_hop_clamps_at_zero() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.prev_hop();
        assert_eq!(app.selected_hop, 0, "should not go below 0");
    }

    #[test]
    fn toggle_pause_flips() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert!(!app.paused);
        app.toggle_pause();
        assert!(app.paused);
        app.toggle_pause();
        assert!(!app.paused);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert!(!app.show_help);
        app.toggle_help();
        assert!(app.show_help);
        app.toggle_help();
        assert!(!app.show_help);
    }

    #[test]
    fn key_q_sets_should_quit() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('q')), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn key_esc_sets_should_quit() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Esc), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_sets_should_quit() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        let ctrl_c = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        handle_key_event(ctrl_c, &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_in_input_mode_quits() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.input_buffer = "8.8.8".to_string();
        let ctrl_c = KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        handle_key_event(ctrl_c, &mut app, 5);
        assert!(app.should_quit);
        assert_ne!(app.input_buffer, "8.8.8c", "should quit, not type 'c'");
    }

    #[test]
    fn key_down_selects_next_target() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        assert_eq!(app.active_target, 0);
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        assert_eq!(app.active_target, 1);
    }

    #[test]
    fn key_up_selects_previous_target() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.active_target = 2;
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        assert_eq!(app.active_target, 1);
    }

    #[test]
    fn key_j_increments_hop() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('j')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_k_decrements_hop() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.selected_hop = 2;
        handle_key_event(press(KeyCode::Char('k')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_p_toggles_pause() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(app.paused);
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(!app.paused);
    }

    #[test]
    fn key_h_toggles_help() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('h')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn key_question_mark_toggles_help() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('?')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn key_g_toggles_chart() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert!(app.show_chart);
        handle_key_event(press(KeyCode::Char('g')), &mut app, 5);
        assert!(!app.show_chart);
        handle_key_event(press(KeyCode::Char('g')), &mut app, 5);
        assert!(app.show_chart);
    }

    #[test]
    fn key_r_sets_reset_requested() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert!(!app.reset_requested);
        handle_key_event(press(KeyCode::Char('r')), &mut app, 5);
        assert!(app.reset_requested);
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
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
                error: None,
            };
            hop.add_probe(probe, 50);
        }
        hop
    }

    fn make_states(count: usize) -> Vec<Arc<RwLock<TraceState>>> {
        (0..count)
            .map(|_| Arc::new(RwLock::new(make_trace_state())))
            .collect()
    }

    #[test]
    fn render_frame_empty_state_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        let state = states[0].read();
        let app = AppState::new(1, Arc::new(AtomicBool::new(false)));

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    #[test]
    fn render_frame_with_hops_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        {
            let mut state = states[0].write();
            state.hops.push(make_hop_with_samples(1));
            state.hops.push(make_hop_with_samples(2));
            state.hops.push(make_hop_with_samples(3));
        }
        let state = states[0].read();
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.selected_hop = 1;

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    #[test]
    fn render_frame_with_help_overlay_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        let state = states[0].read();
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.show_help = true;

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    #[test]
    fn render_frame_short_terminal_skips_chart() {
        let backend = TestBackend::new(80, 15); // below MIN_HEIGHT_FOR_CHART
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        {
            let mut state = states[0].write();
            state.hops.push(make_hop_with_samples(1));
        }
        let state = states[0].read();
        let app = AppState::new(1, Arc::new(AtomicBool::new(false)));

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    // --- target selection tests ---

    #[test]
    fn arrow_down_cycles_target_forward() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        assert_eq!(app.active_target, 0);
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        assert_eq!(app.active_target, 1);
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        assert_eq!(app.active_target, 2);
    }

    #[test]
    fn arrow_up_cycles_target_backward() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.active_target = 2;
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        assert_eq!(app.active_target, 1);
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        assert_eq!(app.active_target, 0);
    }

    #[test]
    fn target_selection_wraps_forward() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.active_target = 2;
        app.next_target();
        assert_eq!(app.active_target, 0, "should wrap from last to first");
    }

    #[test]
    fn target_selection_wraps_backward() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.prev_target();
        assert_eq!(app.active_target, 2, "should wrap from first to last");
    }

    #[test]
    fn target_switch_resets_selected_hop() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.selected_hop = 5;
        app.next_target();
        assert_eq!(app.selected_hop, 0, "switching target should reset hop selection");
    }

    #[test]
    fn single_target_arrows_stay_at_zero() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.next_target();
        assert_eq!(app.active_target, 0);
        app.prev_target();
        assert_eq!(app.active_target, 0);
    }

    #[test]
    fn tab_does_nothing() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Tab), &mut app, 5);
        assert_eq!(app.active_target, 0, "Tab should not change target");
        assert_eq!(app.selected_hop, 0, "Tab should not change hop");
    }

    // --- input mode tests ---

    #[test]
    fn key_a_enters_add_target_mode() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        assert_eq!(app.input_mode, InputMode::Normal);
        handle_key_event(press(KeyCode::Char('a')), &mut app, 5);
        assert_eq!(app.input_mode, InputMode::AddTarget);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn input_mode_esc_cancels() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.input_buffer = "8.8.8".to_string();
        handle_key_event(press(KeyCode::Esc), &mut app, 5);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.input_buffer.is_empty());
        assert!(!app.should_quit, "Esc in input mode should not quit");
    }

    #[test]
    fn input_mode_chars_append_to_buffer() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        handle_key_event(press(KeyCode::Char('1')), &mut app, 5);
        handle_key_event(press(KeyCode::Char('.')), &mut app, 5);
        handle_key_event(press(KeyCode::Char('1')), &mut app, 5);
        assert_eq!(app.input_buffer, "1.1");
    }

    #[test]
    fn input_mode_backspace_removes_last_char() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.input_buffer = "8.8.8".to_string();
        handle_key_event(press(KeyCode::Backspace), &mut app, 5);
        assert_eq!(app.input_buffer, "8.8.");
    }

    #[test]
    fn input_mode_backspace_on_empty_is_noop() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        handle_key_event(press(KeyCode::Backspace), &mut app, 5);
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn input_mode_enter_with_empty_buffer_does_nothing() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        handle_key_event(press(KeyCode::Enter), &mut app, 5);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.pending_target.is_none());
    }

    #[test]
    fn input_mode_enter_sets_pending_target() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.input_buffer = "1.1.1.1".to_string();
        handle_key_event(press(KeyCode::Enter), &mut app, 5);
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.pending_target, Some("1.1.1.1".to_string()));
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn input_mode_ignores_navigation_keys() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.selected_hop = 2;
        let initial_target = app.active_target;
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        handle_key_event(press(KeyCode::Tab), &mut app, 5);
        assert_eq!(app.selected_hop, 2, "navigation should be ignored in input mode");
        assert_eq!(app.active_target, initial_target);
    }

    #[test]
    fn input_mode_q_does_not_quit() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        handle_key_event(press(KeyCode::Char('q')), &mut app, 5);
        assert!(!app.should_quit, "q in input mode should type 'q', not quit");
        assert_eq!(app.input_buffer, "q");
    }

    // --- remove target tests ---

    #[test]
    fn key_d_sets_remove_requested() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('d')), &mut app, 5);
        assert!(app.remove_requested);
    }

    #[test]
    fn key_x_sets_remove_requested() {
        let mut app = AppState::new(3, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('x')), &mut app, 5);
        assert!(app.remove_requested);
    }

    #[test]
    fn key_d_ignored_with_single_target() {
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        handle_key_event(press(KeyCode::Char('d')), &mut app, 5);
        assert!(!app.remove_requested, "cannot remove last target");
    }

    // --- render with input mode ---

    #[test]
    fn render_frame_input_mode_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        let state = states[0].read();
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.input_mode = InputMode::AddTarget;
        app.input_buffer = "8.8.8.8".to_string();

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    #[test]
    fn render_frame_status_message_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(1);
        let state = states[0].read();
        let mut app = AppState::new(1, Arc::new(AtomicBool::new(false)));
        app.status_message = Some("Failed to add target: DNS error".to_string());

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }

    // --- multi-target render tests ---

    #[test]
    fn render_frame_with_multiple_targets_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let states = make_states(3);
        let state = states[0].read();
        let app = AppState::new(3, Arc::new(AtomicBool::new(false)));

        terminal
            .draw(|frame| {
                render_frame(frame, &states, &state, &app, &crate::config::Thresholds::default());
            })
            .unwrap();
    }
}
