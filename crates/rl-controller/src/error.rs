//! Agent-side errors (docs/08). The reward and persistence layers never
//! panic: malformed feedback or disk hiccups degrade to logged no-ops.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RlError {
    #[error("RL clamp/veto: {0}")]
    ActionRejected(&'static str),
    #[error("policy frozen after {0} consecutive vetoes; restore snapshot to unfreeze")]
    Frozen(u16),
    #[error("policy is paused until {0})")]
    Paused(chrono::DateTime<chrono::Utc>),
    #[error("bad context or bucket request: {0}")]
    BadArm(&'static str),
    #[error("snapshot store: {0}")]
    Persistence(String),
    #[error("decompressed snapshot is not parseable: {0}")]
    SnapshotCorrupt(String),
}
