//! Layer E: zero-touch onboarding state machine, 90-day ROI ledger and
//! dioxus/WASM shell (wasm target only).

pub mod ledger;
pub mod onboarding;
pub mod scorecard;

#[cfg(target_family = "wasm")]
#[path = "ui.rs"]
pub mod ui;

