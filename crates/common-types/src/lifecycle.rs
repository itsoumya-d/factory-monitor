use serde::{Deserialize, Serialize};

/// System lifecycle states, persisted in `calibration_state` (docs/02 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LifecycleState {
    Standby,
    Calibrating,
    Counting,
    Correcting,
    Degraded,
}

impl LifecycleState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Standby => "Standby",
            Self::Calibrating => "Calibrating",
            Self::Counting => "Counting",
            Self::Correcting => "Correcting",
            Self::Degraded => "Degraded",
        }
    }

    /// The transition matrix from docs/02 §7. `Degraded` can be entered from
    /// any state, and a manual ack may restore any operational state.
    pub fn is_valid_transition(from: Self, to: Self) -> bool {
        use LifecycleState::*;
        match (from, to) {
            (from, Degraded) if from != Degraded => true,
            (Degraded, to) if to != Degraded => true,
            (Standby, Calibrating) => true,
            (Calibrating, Standby) => false,
            (Calibrating, Counting) => true,
            (Calibrating, Calibrating) => true,
            (Counting, Standby) => true,
            (Counting, Correcting) => true,
            (Correcting, Counting) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn documented_transition_matrix() {
        assert!(LifecycleState::is_valid_transition(
            LifecycleState::Standby,
            LifecycleState::Calibrating
        ));
        assert!(LifecycleState::is_valid_transition(
            LifecycleState::Calibrating,
            LifecycleState::Counting
        ));
        assert!(LifecycleState::is_valid_transition(
            LifecycleState::Counting,
            LifecycleState::Correcting
        ));
        assert!(LifecycleState::is_valid_transition(
            LifecycleState::Correcting,
            LifecycleState::Counting
        ));
        assert!(LifecycleState::is_valid_transition(
            LifecycleState::Counting,
            LifecycleState::Standby
        ));
    }

    #[test]
    fn degraded_is_reachable_from_every_state_and_recovers_manually() {
        for state in [
            LifecycleState::Standby,
            LifecycleState::Calibrating,
            LifecycleState::Counting,
            LifecycleState::Correcting,
        ] {
            assert!(LifecycleState::is_valid_transition(
                state,
                LifecycleState::Degraded
            ));
        }
        for state in [
            LifecycleState::Standby,
            LifecycleState::Calibrating,
            LifecycleState::Counting,
            LifecycleState::Correcting,
        ] {
            assert!(LifecycleState::is_valid_transition(
                LifecycleState::Degraded,
                state
            ));
        }
    }

    #[test]
    fn forbidden_jumps() {
        assert!(!LifecycleState::is_valid_transition(
            LifecycleState::Standby,
            LifecycleState::Counting
        ));
        assert!(!LifecycleState::is_valid_transition(
            LifecycleState::Correcting,
            LifecycleState::Standby
        ));
        assert!(!LifecycleState::is_valid_transition(
            LifecycleState::Standby,
            LifecycleState::Correcting
        ));
        assert!(!LifecycleState::is_valid_transition(
            LifecycleState::Degraded,
            LifecycleState::Degraded
        ));
    }

    #[test]
    fn serde_reflectivity() {
        assert_eq!(
            serde_json::from_str::<LifecycleState>("\"Counting\"").unwrap(),
            LifecycleState::Counting
        );
        assert_eq!(
            serde_json::to_string(&LifecycleState::Calibrating).unwrap(),
            "\"Calibrating\""
        );
    }
}
