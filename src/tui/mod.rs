pub mod widgets;

use std::io::Stdout;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio_util::sync::CancellationToken;

use crate::trace::state::TraceState;

/// TUI application state, separate from trace data.
pub struct AppState {
    pub selected_hop: usize,
    pub paused: bool,
    pub show_help: bool,
    pub should_quit: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            selected_hop: 0,
            paused: false,
            show_help: false,
            should_quit: false,
        }
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
        KeyCode::Char('r') => {} // reserved: reset stats
        KeyCode::Char('g') => {} // reserved: graph toggle
        _ => {}
    }
}

const TICK_RATE: Duration = Duration::from_millis(67); // ~15fps

/// Main TUI event loop.
pub async fn run_tui(
    state: Arc<RwLock<TraceState>>,
    cancel: CancellationToken,
) -> Result<(), anyhow::Error> {
    let mut terminal = setup_terminal()?;
    let mut app = AppState::new();

    let result = run_event_loop(&mut terminal, &mut app, &state, &cancel).await;

    restore_terminal(&mut terminal)?;
    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
    state: &Arc<RwLock<TraceState>>,
    cancel: &CancellationToken,
) -> Result<(), anyhow::Error> {
    let mut tick_interval = tokio::time::interval(TICK_RATE);

    loop {
        // Render
        let hop_count = state.read().unwrap().hop_count();
        let target = state.read().unwrap().target.clone();
        let round = state.read().unwrap().round;

        terminal.draw(|frame| {
            let area = frame.area();
            let text = format!(
                "pplot - tracing {} | round {} | hops: {} | hop: {} | {}",
                target,
                round,
                hop_count,
                app.selected_hop,
                if app.paused { "PAUSED" } else { "running" },
            );
            frame.render_widget(
                ratatui::widgets::Paragraph::new(text),
                area,
            );
        })?;

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
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

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
        let app = AppState::new();
        assert_eq!(app.selected_hop, 0);
        assert!(!app.paused);
        assert!(!app.show_help);
        assert!(!app.should_quit);
    }

    #[test]
    fn next_hop_increments_within_bounds() {
        let mut app = AppState::new();
        app.next_hop(5);
        assert_eq!(app.selected_hop, 1);
        app.next_hop(5);
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn next_hop_clamps_at_max() {
        let mut app = AppState::new();
        app.selected_hop = 4;
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4, "should not exceed max - 1");
        app.next_hop(5);
        assert_eq!(app.selected_hop, 4);
    }

    #[test]
    fn next_hop_zero_max_is_noop() {
        let mut app = AppState::new();
        app.next_hop(0);
        assert_eq!(app.selected_hop, 0);
    }

    #[test]
    fn prev_hop_decrements() {
        let mut app = AppState::new();
        app.selected_hop = 3;
        app.prev_hop();
        assert_eq!(app.selected_hop, 2);
    }

    #[test]
    fn prev_hop_clamps_at_zero() {
        let mut app = AppState::new();
        app.prev_hop();
        assert_eq!(app.selected_hop, 0, "should not go below 0");
    }

    #[test]
    fn toggle_pause_flips() {
        let mut app = AppState::new();
        assert!(!app.paused);
        app.toggle_pause();
        assert!(app.paused);
        app.toggle_pause();
        assert!(!app.paused);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = AppState::new();
        assert!(!app.show_help);
        app.toggle_help();
        assert!(app.show_help);
        app.toggle_help();
        assert!(!app.show_help);
    }

    #[test]
    fn key_q_sets_should_quit() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Char('q')), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn key_esc_sets_should_quit() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Esc), &mut app, 5);
        assert!(app.should_quit);
    }

    #[test]
    fn key_down_increments_hop() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Down), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_j_increments_hop() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Char('j')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_up_decrements_hop() {
        let mut app = AppState::new();
        app.selected_hop = 2;
        handle_key_event(press(KeyCode::Up), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_k_decrements_hop() {
        let mut app = AppState::new();
        app.selected_hop = 2;
        handle_key_event(press(KeyCode::Char('k')), &mut app, 5);
        assert_eq!(app.selected_hop, 1);
    }

    #[test]
    fn key_p_toggles_pause() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(app.paused);
        handle_key_event(press(KeyCode::Char('p')), &mut app, 5);
        assert!(!app.paused);
    }

    #[test]
    fn key_h_toggles_help() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Char('h')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn key_question_mark_toggles_help() {
        let mut app = AppState::new();
        handle_key_event(press(KeyCode::Char('?')), &mut app, 5);
        assert!(app.show_help);
    }

    #[test]
    fn release_events_are_ignored() {
        let mut app = AppState::new();
        let release = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        handle_key_event(release, &mut app, 5);
        assert!(!app.should_quit, "release events should be ignored");
    }
}
