//! esp-crab coherent dual-chip alignment planning (docs: dual ESP32 pair
//! on one crystal for phase-difference AoA/VitalSigns sensing).

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PhaseSyncError {
    #[error("packet rate must be in [250, 1000] Hz, got {0}")]
    PacketRateOutOfRange(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairPlan {
    /// Coherent csi requires sub-period jitter; budget = period/10,000.
    pub jitter_budget_ns: u32,
    pub pkt_rate_hz: u16,
}

pub fn plan(pkt_rate_hz: u16) -> Result<PairPlan, PhaseSyncError> {
    if !(250..=1000).contains(&pkt_rate_hz) {
        return Err(PhaseSyncError::PacketRateOutOfRange(pkt_rate_hz));
    }
    let period_ns = 1_000_000_000_u32 / u32::from(pkt_rate_hz);
    Ok(PairPlan {
        jitter_budget_ns: period_ns / 10_000,
        pkt_rate_hz,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn one_kiloherz_gives_hundred_nanosecond_budget() {
        let p = plan(1000).expect("1 kHz supported");
        assert_eq!(p.jitter_budget_ns, 100);
        assert_eq!(p.pkt_rate_hz, 1000);
    }

    #[test]
    fn rate_bounds() {
        assert_eq!(
            plan(100).expect_err("below floor"),
            PhaseSyncError::PacketRateOutOfRange(100)
        );
        assert!(plan(250).is_ok());
        assert!(plan(999).is_ok());
        assert!(plan(1500).is_err());
    }
}
