//! Ingestion error taxonomy (docs/05 §3): fail-at-open imports, reject bad
//! payloads with exact byte counts, defer hardware-phase formats.

#[derive(Debug, thiserror::Error)]
pub enum CsiError {
    #[error("CSI payload for format {format} needs {expected} bytes but carries {got}")]
    InvalidPayload {
        format: String,
        expected: usize,
        got: usize,
    },
    #[error("CSI format {0:?} is unsupported until the hardware phase (docs/15 §2 Phase 8)")]
    Unsupported(crate::record::CsiFormat),
    #[error("NPY parse failure: {0}")]
    CorruptNpy(String),
    #[error("window config error: {0}")]
    InvalidConfig(String),
}

impl CsiError {
    pub fn invalid_payload(format: impl std::fmt::Display, expected: usize, got: usize) -> Self {
        Self::InvalidPayload {
            format: format.to_string(),
            expected,
            got,
        }
    }
}
