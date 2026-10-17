//! Crate error taxonomy. Library crates use `thiserror` (docs/); production
//! paths never panic — validation surfaces as values of this error type.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("field '{field}' value {value} is outside the safe range [{low}, {high}]")]
pub struct OutOfRangeDetail {
    pub field: String,
    pub value: String,
    pub low: f32,
    pub high: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommonTypesError {
    #[error("{0}")]
    OutOfRange(#[from] OutOfRangeDetail),
    #[error("identifier '{0}' must be a non-empty string")]
    InvalidId(String),
    #[error("time window is reversed: ts_from ({ts_from:?}) is after ts_to ({ts_to:?})")]
    ReversedTimeWindow {
        ts_from: chrono::DateTime<chrono::Utc>,
        ts_to: chrono::DateTime<chrono::Utc>,
    },
    #[error(
        "ModalityBlend weights must sum to 1.0 (±0.001) with each side inside [0.05, 0.95]; got vision={vision}, wifi={wifi}"
    )]
    BlendNotNormalized { vision: f32, wifi: f32 },
    #[error("count values must be non-negative (got {0})")]
    NegativeCount(i64),
}

impl CommonTypesError {
    pub fn out_of_range(field: &'static str, value: String, low: f32, high: f32) -> Self {
        Self::OutOfRange(OutOfRangeDetail {
            field: field.to_string(),
            value,
            low,
            high,
        })
    }
}
