//! `vlm-orchestrator` (docs/09): the only crate permitted to leave the
//! building — morning layout diffing, EOD root-cause localization, and
//! auto-labeling for the nightly fine-tune sit behind one provider seam
//! with circuit-breaking, spend caps and payload guardrails.

pub mod auto_label;
pub mod breaker;
pub mod error;
pub mod guardrails;
pub mod layout_diff;
#[cfg(feature = "fine_tune")]
pub mod nightly;
pub mod orchestrator;
pub mod payload;
pub mod provider;
pub mod root_cause;
pub mod schema;
#[cfg(test)]
pub(crate) mod testutil;

pub use error::VlmError;
pub use layout_diff::{Floorplan, LayoutChangePlan, MorningBlock};
pub use orchestrator::{NoSleep, OrchestratorDeps, TokioSleeper, VlmOrchestrator};
pub use provider::{
    CloudProvider, HttpResponse, HttpTransport, MockProvider, ProviderKind, VlmProvider, VlmRequest,
};
pub use root_cause::{
    CocoBox, EventRow, FailureProfile, FrameSample, MinuteWindow, RootCauseReport,
};
pub use schema::{JsonSchemaSpec, layout_diff_schema, root_cause_schema};
