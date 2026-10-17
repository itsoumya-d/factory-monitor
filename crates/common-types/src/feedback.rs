//! EOD human-in-the-loop correction payload (docs/06 §4) feeding both the RL
//! reward computation (docs/08 §4) and the nightly auto-label pipeline
//! (docs/09 §3).

use bitflags::bitflags;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CommonTypesError;
use crate::ids::{OperatorId, ZoneId};

bitflags! {
    /// Operator-classified failure mode; serialized as the `SMALLINT`
    /// `failure_tags` of `audit_corrections` (docs/11 §1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FailureTag: u16 {
        const MISSED_COUNT = 1 << 0;
        const DOUBLE_COUNT = 1 << 1;
        const PALLET_AS_BOX = 1 << 2;
        const OCCLUSION_LIGHTING = 1 << 3;
        const OTHER = 1 << 4;
    }
}

impl FailureTag {
    pub const fn to_db_mask(&self) -> i16 {
        self.bits() as i16
    }

    pub const fn from_db_mask(mask: i16) -> Self {
        Self::from_bits_truncate(mask as u16)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanAuditFeedback {
    pub zone_id: ZoneId,
    pub reported_count: i64,
    pub actual_count: i64,
    pub ts_from: DateTime<Utc>,
    pub ts_to: DateTime<Utc>,
    pub failure_tags: FailureTag,
    pub notes: Option<String>,
    pub operator_id: OperatorId,
    /// Double-confirmed opt-in before the heavyweight VLM retrain path runs.
    pub wants_retrain: bool,
}

impl HumanAuditFeedback {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        zone_id: ZoneId,
        reported_count: i64,
        actual_count: i64,
        ts_from: DateTime<Utc>,
        ts_to: DateTime<Utc>,
        failure_tags: FailureTag,
        notes: Option<String>,
        operator_id: OperatorId,
        wants_retrain: bool,
    ) -> Result<Self, CommonTypesError> {
        if reported_count < 0 || actual_count < 0 {
            return Err(CommonTypesError::NegativeCount(
                reported_count.min(actual_count),
            ));
        }
        if ts_to < ts_from {
            return Err(CommonTypesError::ReversedTimeWindow { ts_from, ts_to });
        }
        Ok(Self {
            zone_id,
            reported_count,
            actual_count,
            ts_from,
            ts_to,
            failure_tags,
            notes,
            operator_id,
            wants_retrain,
        })
    }

    /// Signed error magnitude: `actual - reported`. Positive = undercounted.
    pub const fn count_error(&self) -> i64 {
        self.actual_count - self.reported_count
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::TimeZone;

    fn ts(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
    }

    fn sample(reported: i64, actual: i64) -> Result<HumanAuditFeedback, CommonTypesError> {
        HumanAuditFeedback::try_new(
            ZoneId::try_new("line_4")?,
            reported,
            actual,
            ts(14),
            ts(16),
            FailureTag::PALLET_AS_BOX | FailureTag::DOUBLE_COUNT,
            Some("Camera counted empty pallets as boxes during the afternoon shift".into()),
            OperatorId::try_new("plant-7")?,
            true,
        )
    }

    #[test]
    fn happy_path_with_signed_error() {
        let feedback = sample(1485, 1420).unwrap();
        assert_eq!(feedback.count_error(), -65);
        assert_eq!(feedback.failure_tags.to_db_mask(), 0b0110);
    }

    #[test]
    fn negative_counts_rejected() {
        assert!(sample(1485, -1).is_err());
        assert!(sample(-3, 100).is_err());
    }

    #[test]
    fn reversed_window_rejected() {
        let built = HumanAuditFeedback::try_new(
            ZoneId::try_new("line_4").unwrap(),
            100,
            100,
            ts(16),
            ts(14),
            FailureTag::empty(),
            None,
            OperatorId::try_new("plant-7").unwrap(),
            false,
        );
        assert!(built.is_err());
    }

    #[test]
    fn tags_serde_roundtrip_db_mask() {
        let tags = FailureTag::PALLET_AS_BOX | FailureTag::MISSED_COUNT;
        let json = serde_json::to_string(&tags).unwrap();
        let back: FailureTag = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tags);
        assert_eq!(FailureTag::from_db_mask(tags.to_db_mask()), tags);
    }

    #[test]
    fn feedback_serde_roundtrip() {
        let feedback = sample(1485, 1420).unwrap();
        let json = serde_json::to_string(&feedback).unwrap();
        let back: HumanAuditFeedback = serde_json::from_str(&json).unwrap();
        assert_eq!(back, feedback);
    }
}
