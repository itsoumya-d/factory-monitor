//! Subject-level data aggregation: the ONLY way person-linked entities
//! move through the platform is as station/zone-scoped pseudonyms
//! (docs/15 "station aggregation" invariant). Raw global identities and
//! raw RGB die in RAM.

use sha2::{Digest, Sha256};

/// Station-local pseudonym; identity can never be re-linked across
/// stations by design (double hashing inside a station salt).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AgentKey {
    pub station_id: String,
    /// hex(SHA256(station_salt || person_world_hash))[..32]
    pub pseudonym: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AggregationError {
    #[error("station id must not be empty")]
    EmptyStation,
    #[error("person world hash must be provided")]
    MissingPersonHash,
}

/// Derive the station-local pseudonym for a person.
pub fn station_pseudonym(
    station_id: &str,
    station_salt: &str,
    person_world_hash: &str,
) -> Result<AgentKey, AggregationError> {
    if station_id.is_empty() {
        return Err(AggregationError::EmptyStation);
    }
    if person_world_hash.is_empty() {
        return Err(AggregationError::MissingPersonHash);
    }
    let mut h = Sha256::new();
    h.update(station_salt.as_bytes());
    h.update(b"|");
    h.update(person_world_hash.as_bytes());
    let digest = h.finalize();
    let hex = digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<String>();
    Ok(AgentKey {
        station_id: station_id.to_string(),
        pseudonym: hex[..32].to_string(),
    })
}

/// KPI-facing accessor: the only fields that may ever leave a station are
/// the station id and the KPI values themselves.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZoneAggregate {
    pub station_id: String,
    pub zone_id: String,
    pub value: f64,
    pub window_ms: u64,
}

impl ZoneAggregate {
    /// Invariant check: aggregates must be station-scoped; a global
    /// (`station_id` empty) record must never be emitted.
    pub fn emit_ready(&self) -> bool {
        !self.station_id.is_empty() && !self.zone_id.is_empty() && self.window_ms > 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn same_person_different_stations_are_unlinkable() {
        let a = station_pseudonym("line1", "salt-a", "person-xyz").unwrap();
        let b = station_pseudonym("aisle3", "salt-b", "person-xyz").unwrap();
        assert_ne!(a.pseudonym, b.pseudonym);
        assert_eq!(a.station_id, "line1");
    }

    #[test]
    fn pseudonym_stable_within_station() {
        let a1 = station_pseudonym("line1", "salt-a", "person-xyz").unwrap();
        let a2 = station_pseudonym("line1", "salt-a", "person-xyz").unwrap();
        assert_eq!(a1, a2);
    }

    #[test]
    fn empty_station_or_person_rejected() {
        assert!(station_pseudonym("", "salt", "p").is_err());
        assert!(station_pseudonym("line1", "salt", "").is_err());
    }

    #[test]
    fn zone_aggregate_requires_station_scope() {
        let global = ZoneAggregate {
            station_id: "".into(),
            zone_id: "z".into(),
            value: 1.0,
            window_ms: 1_000,
        };
        assert!(!global.emit_ready());
        let scoped = ZoneAggregate {
            station_id: "line1".into(),
            zone_id: "z".into(),
            value: 1.0,
            window_ms: 1_000,
        };
        assert!(scoped.emit_ready());
    }
}
