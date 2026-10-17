//! Annex III safeguards kept outside Art 5: biometric prohibitions and
//! volatile-RGB data minimization.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Annex3Error {
    #[error("1:N biometric identification (watchlists of many identities) is prohibited")]
    OneToManyProhibited,
    #[error("1:1 verification requires an explicit consent handle")]
    ConsentMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiometricScan {
    /// 1:1 — match against one consented identity.
    Verify { consent_handle: Option<String> },
    /// 1:N — search identity across the whole registry.
    Identify,
}

impl BiometricScan {
    pub fn is_permitted(&self) -> Result<(), Annex3Error> {
        match self {
            Self::Identify => Err(Annex3Error::OneToManyProhibited),
            Self::Verify {
                consent_handle: None,
            } => Err(Annex3Error::ConsentMissing),
            Self::Verify {
                consent_handle: Some(handle),
            } if handle.trim().is_empty() => Err(Annex3Error::ConsentMissing),
            Self::Verify {
                consent_handle: Some(_),
            } => Ok(()),
        }
    }
}

/// Data minimization state machine for camera frames (docs/04 pipeline):
/// frames enter as raw RGB (RAM), are reduced to tracks, and only certain
/// artifacts may persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RgbFrame {
    pub frame_id: u64,
    /// Payload marker; real bytes never enter the persisted layer.
    #[serde(skip)]
    pub ram_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedArtifact {
    /// Structured track states — allowed.
    TrackState,
    /// Low-res JPEG bound to an alert — allowed.
    AlertSnapshot,
    /// Full RGB frames — never persisted; audit tooling must catch it.
    RawRgb,
}

impl PersistedArtifact {
    pub fn may_persist(self) -> bool {
        !matches!(self, Self::RawRgb)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn biometric_matrix() {
        assert_eq!(
            BiometricScan::Identify.is_permitted(),
            Err(Annex3Error::OneToManyProhibited)
        );
        assert_eq!(
            BiometricScan::Verify {
                consent_handle: None
            }
            .is_permitted(),
            Err(Annex3Error::ConsentMissing)
        );
        assert!(
            BiometricScan::Verify {
                consent_handle: Some("consent-1".into())
            }
            .is_permitted()
            .is_ok()
        );
    }

    #[test]
    fn raw_rgb_never_persists() {
        assert!(!PersistedArtifact::RawRgb.may_persist());
        assert!(PersistedArtifact::TrackState.may_persist());
        assert!(PersistedArtifact::AlertSnapshot.may_persist());
    }
}
