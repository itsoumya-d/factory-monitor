//! Error taxonomy for the only crate allowed to leave the building
//! (the cloud AI boundary, docs/09).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VlmError {
    #[error("provider {provider} returned {status}: {body}")]
    Provider {
        provider: String,
        status: u16,
        body: String,
    },
    #[error("provider {provider} rate-limited (429)")]
    RateLimited { provider: String },
    #[error("circuit open, retry in {retry_in_secs}s")]
    CircuitOpen { retry_in_secs: u64 },
    #[error("payload rejected: {0}")]
    InvalidPayload(String),
    #[error("daily attn budget exhausted (soft mode only runs morning + EOD)")]
    BudgetExhausted,
    #[error("response json invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema violation: {0}")]
    Schema(String),
    #[error("http transport failure: {0}")]
    Http(String),
    #[error("auth/secret missing for {0}")]
    Auth(String),
    #[error("timed out calling {0}")]
    Timeout(String),
}

impl VlmError {
    /// Which failures are worth burning 1s/4s/16s retries on (docs/09 §1).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            VlmError::RateLimited { .. } | VlmError::Http { .. } | VlmError::Timeout { .. }
        ) || matches!(self, VlmError::Provider { status, .. } if *status >= 500)
    }
}
