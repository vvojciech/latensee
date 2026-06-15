use ratatui::layout::Alignment;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Static help text listing all keybindings.
pub fn help_text() -> &'static str {
    "\
Keybindings:
  q / Esc    Quit
  Up / k     Previous hop
  Down / j   Next hop
  p          Pause/resume
  h / ?      Toggle this help
  r          Reset statistics
  g          Toggle graph"
}

/// Build a centered `Paragraph` widget with help text and a bordered block.
pub fn help_widget() -> Paragraph<'static> {
    Paragraph::new(help_text())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Help"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_contains_quit() {
        assert!(help_text().contains("Quit"));
    }

    #[test]
    fn help_text_contains_all_keybindings() {
        let text = help_text();
        for key in ["q", "Esc", "Up", "k", "Down", "j", "p", "h", "?", "r", "g"] {
            assert!(
                text.contains(key),
                "help text missing keybinding: {key}"
            );
        }
    }
}
