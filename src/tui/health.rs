use ratatui::style::Color;

use crate::config::Thresholds;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthLevel {
    Good,
    Warning,
    Critical,
    NoData,
}

pub fn rtt_health(last_rtt_ms: Option<f64>, thresholds: &Thresholds) -> HealthLevel {
    match last_rtt_ms {
        None => HealthLevel::NoData,
        Some(ms) if ms >= thresholds.rtt_crit_ms => HealthLevel::Critical,
        Some(ms) if ms >= thresholds.rtt_warn_ms => HealthLevel::Warning,
        Some(_) => HealthLevel::Good,
    }
}

pub fn loss_health(loss_pct: f64, received: u64, thresholds: &Thresholds) -> HealthLevel {
    if received == 0 {
        return HealthLevel::NoData;
    }
    if loss_pct >= thresholds.loss_crit_pct {
        HealthLevel::Critical
    } else if loss_pct >= thresholds.loss_warn_pct {
        HealthLevel::Warning
    } else {
        HealthLevel::Good
    }
}

pub fn combined_health(rtt: HealthLevel, loss: HealthLevel) -> HealthLevel {
    match (rtt, loss) {
        (HealthLevel::Critical, _) | (_, HealthLevel::Critical) => HealthLevel::Critical,
        (HealthLevel::Warning, _) | (_, HealthLevel::Warning) => HealthLevel::Warning,
        (HealthLevel::NoData, _) | (_, HealthLevel::NoData) => HealthLevel::NoData,
        _ => HealthLevel::Good,
    }
}

pub fn health_fg(level: HealthLevel) -> Color {
    match level {
        HealthLevel::Good => Color::Green,
        HealthLevel::Warning => Color::Yellow,
        HealthLevel::Critical => Color::Red,
        HealthLevel::NoData => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Thresholds {
        Thresholds::default()
    }

    #[test]
    fn rtt_health_good() {
        assert_eq!(rtt_health(Some(10.0), &defaults()), HealthLevel::Good);
    }

    #[test]
    fn rtt_health_warn() {
        assert_eq!(rtt_health(Some(80.0), &defaults()), HealthLevel::Warning);
    }

    #[test]
    fn rtt_health_crit() {
        assert_eq!(rtt_health(Some(200.0), &defaults()), HealthLevel::Critical);
    }

    #[test]
    fn rtt_health_at_boundary() {
        assert_eq!(rtt_health(Some(50.0), &defaults()), HealthLevel::Warning);
        assert_eq!(rtt_health(Some(150.0), &defaults()), HealthLevel::Critical);
    }

    #[test]
    fn rtt_health_no_data() {
        assert_eq!(rtt_health(None, &defaults()), HealthLevel::NoData);
    }

    #[test]
    fn loss_health_good() {
        assert_eq!(loss_health(0.0, 10, &defaults()), HealthLevel::Good);
    }

    #[test]
    fn loss_health_warn() {
        assert_eq!(loss_health(3.0, 10, &defaults()), HealthLevel::Warning);
    }

    #[test]
    fn loss_health_crit() {
        assert_eq!(loss_health(10.0, 10, &defaults()), HealthLevel::Critical);
    }

    #[test]
    fn loss_health_no_data() {
        assert_eq!(loss_health(100.0, 0, &defaults()), HealthLevel::NoData);
    }

    #[test]
    fn combined_health_worst_wins() {
        assert_eq!(
            combined_health(HealthLevel::Good, HealthLevel::Critical),
            HealthLevel::Critical
        );
        assert_eq!(
            combined_health(HealthLevel::Warning, HealthLevel::Good),
            HealthLevel::Warning
        );
        assert_eq!(
            combined_health(HealthLevel::Good, HealthLevel::Good),
            HealthLevel::Good
        );
        assert_eq!(
            combined_health(HealthLevel::NoData, HealthLevel::Good),
            HealthLevel::NoData
        );
    }

    #[test]
    fn health_fg_returns_expected_colors() {
        assert_eq!(health_fg(HealthLevel::Good), Color::Green);
        assert_eq!(health_fg(HealthLevel::Warning), Color::Yellow);
        assert_eq!(health_fg(HealthLevel::Critical), Color::Red);
        assert_eq!(health_fg(HealthLevel::NoData), Color::DarkGray);
    }

    #[test]
    fn custom_thresholds() {
        let t = Thresholds {
            rtt_warn_ms: 10.0,
            rtt_crit_ms: 20.0,
            loss_warn_pct: 0.5,
            loss_crit_pct: 2.0,
        };
        assert_eq!(rtt_health(Some(15.0), &t), HealthLevel::Warning);
        assert_eq!(rtt_health(Some(25.0), &t), HealthLevel::Critical);
        assert_eq!(loss_health(1.0, 10, &t), HealthLevel::Warning);
        assert_eq!(loss_health(3.0, 10, &t), HealthLevel::Critical);
    }
}
