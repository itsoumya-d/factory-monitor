//! Capability guardrails: which topology may promise which skill.
//! An overclaiming single node must be incapable of advertising it.

#[cfg(not(test))]
use crate::{String, format};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    SingleNode,
    /// >= 3 nodes within mutual CSI reach.
    Mesh {
        peers: u8,
    },
    /// esp-crab style dual-chip coherent pair.
    DualChipEspCrab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    PersonCounting,
    Triangulation,
    FinePoseEstimation,
    VitalSigns,
    RoomLevelActivity,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityError {
    #[error("mesh needs at least 3 peers, got {0}")]
    MeshTooSmall(u8),
    #[error("capability {capability} is outside the {topology} guard matrix")]
    Unsupported {
        capability: String,
        topology: String,
    },
}

impl Topology {
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if let Self::Mesh { peers } = self
            && *peers < 3
        {
            return Err(CapabilityError::MeshTooSmall(*peers));
        }
        Ok(())
    }

    fn allowed(&self, capability: &Capability) -> bool {
        match self {
            Self::SingleNode => matches!(
                capability,
                Capability::VitalSigns | Capability::RoomLevelActivity
            ),
            Self::Mesh { .. } => matches!(
                capability,
                Capability::PersonCounting | Capability::VitalSigns | Capability::RoomLevelActivity
            ),
            Self::DualChipEspCrab => true,
        }
    }
}

/// A node's advertised capability must be within its topology's guard
/// matrix; per docs this is a sensor-side claim check, not just UI copy.
pub fn claims_within_guardrail(
    topology: &Topology,
    advertised: &[Capability],
) -> Result<(), CapabilityError> {
    topology.validate()?;
    for c in advertised {
        if !topology.allowed(c) {
            return Err(CapabilityError::Unsupported {
                capability: format!("{c:?}"),
                topology: format!("{topology:?}"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn all() -> Vec<Capability> {
        vec![
            Capability::PersonCounting,
            Capability::Triangulation,
            Capability::FinePoseEstimation,
            Capability::VitalSigns,
            Capability::RoomLevelActivity,
        ]
    }

    #[test]
    fn single_node_only_vital_signs_and_activity() {
        let topo = Topology::SingleNode;
        let allowed: Vec<Capability> = all().into_iter().filter(|c| topo.allowed(c)).collect();
        assert_eq!(
            allowed,
            vec![Capability::VitalSigns, Capability::RoomLevelActivity]
        );
        // Counting from one node must never be promised.
        let _ = claims_within_guardrail(&topo, &advertise_only_safe(&topo));
    }

    #[test]
    fn mesh_needs_three_peers() {
        assert_eq!(
            Topology::Mesh { peers: 2 }.validate(),
            Err(CapabilityError::MeshTooSmall(2))
        );
        assert!(Topology::Mesh { peers: 3 }.validate().is_ok());
        let mesh = Topology::Mesh { peers: 4 };
        // Mesh may count but not pose-estimate.
        let overclaim = vec![Capability::FinePoseEstimation];
        assert!(claims_within_guardrail(&mesh, &overclaim).is_err());
    }

    #[test]
    fn esp_crab_unlocks_all() {
        let topo = Topology::DualChipEspCrab;
        assert!(claims_within_guardrail(&topo, &all()).is_ok());
    }

    fn advertise_only_safe(topo: &Topology) -> Vec<Capability> {
        all().into_iter().filter(|c| topo.allowed(c)).collect()
    }
}
