//! Zero-touch onboarding: 4 stages, each with its drift checks
//! (Jetson engines, TPM handle, ESP-NOW lock, discovery, chat-to-ROI,
//! live run) — hard failure whenever any check can't pass facts.

use monitoring_contract::contract::MonitoringContract;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    PlugIn,
    Discovery,
    ChatToContract,
    Operation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteFacts {
    pub jetson_engines_ready: bool,
    pub tpm_handle_ready: bool,
    pub espnow_lock: Option<(u16, u16)>, // (channel, pkt_rate_hz)
    pub discovery_cameras: usize,
    /// Chat session produced a draft contract.
    pub draft: Option<MonitoringContract>,
}

impl SiteFacts {
    pub fn sync_and_ready(&self) -> bool {
        self.jetson_engines_ready
            && self.tpm_handle_ready
            && self.espnow_lock.is_some()
            && self.discovery_cameras > 0
            && self.draft.is_some()
    }
}

impl SiteFacts {
    pub fn primitives_ready(&self) -> bool {
        self.jetson_engines_ready && self.tpm_handle_ready && self.espnow_lock.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OnboardingError {
    #[error("jetson engines were not synced during the plug-in stage")]
    JetsonNotReady,
    #[error("TPM handle missing")]
    TpmMissing,
    #[error("ESP-NOW lock has no channel/rate; full-body agreement required")]
    EspNowUnlocked,
    #[error("no cameras discovered at all")]
    NoCameras,
    #[error("chat-to-contract is incomplete; facts: {0}")]
    DraftNotReady(&'static str),
    #[error("stage transition denied: from {0:?} to {1:?}")]
    IllegalTransition(Stage, Stage),
}

/// Checks the facts per stage; truth-in-advertising for the UI flow.
pub fn check_stage(stage: Stage, facts: &SiteFacts) -> Result<(), OnboardingError> {
    match stage {
        Stage::PlugIn => {
            if !facts.jetson_engines_ready {
                return Err(OnboardingError::JetsonNotReady);
            }
            if !facts.tpm_handle_ready {
                return Err(OnboardingError::TpmMissing);
            }
            if facts.espnow_lock.is_none() {
                return Err(OnboardingError::EspNowUnlocked);
            }
            Ok(())
        }
        Stage::Discovery => {
            if facts.discovery_cameras == 0 {
                return Err(OnboardingError::NoCameras);
            }
            Ok(())
        }
        Stage::ChatToContract | Stage::Operation => {
            if facts.draft.is_none() {
                return Err(OnboardingError::DraftNotReady(
                    "chat stage must produce a draft",
                ));
            }
            if !facts.sync_and_ready() {
                return Err(OnboardingError::NoCameras);
            }
            Ok(())
        }
    }
}

/// Stage advancement is one-way only (PlugIn → … → Operation).
pub fn advance(current: Stage) -> Result<Stage, OnboardingError> {
    let next = match current {
        Stage::PlugIn => Stage::Discovery,
        Stage::Discovery => Stage::ChatToContract,
        Stage::ChatToContract => Stage::Operation,
        Stage::Operation => {
            return Err(OnboardingError::IllegalTransition(
                Stage::Operation,
                Stage::Operation,
            ));
        }
    };
    Ok(next)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use monitoring_contract::templates;

    fn race() -> MonitoringContract {
        templates::qsr_drivethru().expect("fixture")
    }

    fn complete_facts() -> SiteFacts {
        SiteFacts {
            jetson_engines_ready: true,
            tpm_handle_ready: true,
            espnow_lock: Some((2412, 1000)),
            discovery_cameras: 3,
            draft: Some(race()),
        }
    }

    #[test]
    fn plugin_stage_checks_primitives() {
        let worst = SiteFacts {
            jetson_engines_ready: false,
            tpm_handle_ready: false,
            espnow_lock: None,
            discovery_cameras: 0,
            draft: None,
        };
        assert!(check_stage(Stage::PlugIn, &worst).is_err());
        assert!(check_stage(Stage::PlugIn, &complete_facts()).is_ok());
    }

    #[test]
    fn discovery_only_needs_cameras() {
        let mut f = complete_facts();
        f.draft = None;
        assert!(check_stage(Stage::Discovery, &f).is_ok());
        f.discovery_cameras = 0;
        assert!(check_stage(Stage::Discovery, &f).is_err());
    }

    #[test]
    fn chat_and_operation_require_everything() {
        let mut f = complete_facts();
        f.draft = None;
        assert!(check_stage(Stage::ChatToContract, &f).is_err());
        assert!(check_stage(Stage::Operation, &complete_facts()).is_ok());
    }

    #[test]
    fn stage_advancement_is_one_way_chain() {
        assert_eq!(advance(Stage::PlugIn), Ok(Stage::Discovery));
        assert_eq!(advance(Stage::Discovery), Ok(Stage::ChatToContract));
        assert_eq!(advance(Stage::ChatToContract), Ok(Stage::Operation));
        assert_eq!(
            advance(Stage::Operation),
            Err(OnboardingError::IllegalTransition(
                Stage::Operation,
                Stage::Operation
            ))
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod facts_tests {
    use super::*;

    #[test]
    fn fact_helpers_track_independence() {
        let f = SiteFacts {
            jetson_engines_ready: true,
            tpm_handle_ready: false,
            espnow_lock: Some((2412, 1000)),
            discovery_cameras: 0,
            draft: None,
        };
        assert!(!f.primitives_ready(), "tpm missing blocks primitives");
        assert!(!f.sync_and_ready(), "absent cameras/draft block sync");
    }
}
