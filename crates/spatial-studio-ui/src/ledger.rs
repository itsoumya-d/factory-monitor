//! 90-day ROI ledger: deterministic earnings across the four channels
//! weighed against appliance amortization + energy spend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RoiChannels {
    /// Value recovered from faster turnaround/throughput events.
    pub revenue_uplift: f64,
    /// Labour-hours reclaimed (hours × loaded hourly cost).
    pub labor_saved: f64,
    /// Losses avoided (downtime + backlog + mistakes caught early).
    pub losses_avoided: f64,
    /// Energy the appliance consumed over the quarter (kWh × tariff).
    pub energy_cost: f64,
}

impl RoiChannels {
    pub fn net(self, appliance_amortization: f64) -> f64 {
        self.revenue_uplift + self.labor_saved + self.losses_avoided
            - self.energy_cost
            - appliance_amortization
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LedgerWindow {
    pub start_day: u32,
    pub end_day: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerError {
    #[error("window must be 1..=90 days, got {0}")]
    WindowOutOfBounds(u32),
}

pub fn validate_window(days: u32) -> Result<(), LedgerError> {
    if (1..=90).contains(&days) {
        Ok(())
    } else {
        Err(LedgerError::WindowOutOfBounds(days))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn net_is_channels_minus_costs() {
        let r = RoiChannels {
            revenue_uplift: 1200.0,
            labor_saved: 400.0,
            losses_avoided: 250.0,
            energy_cost: 30.0,
        };
        assert!((r.net(90.0) - (1200.0 + 400.0 + 250.0 - 30.0 - 90.0)).abs() < 1e-9);
    }

    #[test]
    fn window_is_quarter_bounded() {
        assert!(validate_window(90).is_ok());
        assert_eq!(validate_window(91), Err(LedgerError::WindowOutOfBounds(91)));
        assert_eq!(validate_window(0), Err(LedgerError::WindowOutOfBounds(0)));
    }
}
