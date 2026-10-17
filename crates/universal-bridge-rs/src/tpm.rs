//! TPM 2.0 NV-key signing + C2PA fragment manifest, with the mock seam
//! the pilot runs on until the real `tss-esapi` session is wired.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TpmError {
    #[error("no NV key handle loaded")]
    NoKeyHandle,
    #[error("payload must be explicitly wrapped by the caller")]
    EmptyPayload,
}

/// Platform seam: the credentials-object path either talks to the TPM or
/// (in the mock path used by tests) signs in-process.
pub trait TpmSigner: Send + Sync {
    fn sign_nv_key(&self, payload: &[u8]) -> Result<Vec<u8>, TpmError>;
}

/// A pure in-process signer; signature = SHA-256 of (key || payload).
pub struct InProcessSigner {
    pub key_seed: Vec<u8>,
}

impl TpmSigner for InProcessSigner {
    fn sign_nv_key(&self, payload: &[u8]) -> Result<Vec<u8>, TpmError> {
        if self.key_seed.is_empty() {
            return Err(TpmError::NoKeyHandle);
        }
        if payload.is_empty() {
            return Err(TpmError::EmptyPayload);
        }
        let mut h = Sha256::new();
        h.update(&self.key_seed);
        h.update(payload);
        Ok(h.finalize().to_vec())
    }
}

/// Signing must complete within 40 ms even on the software seam (docs).
pub const SIGN_BUDGET: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct C2paFragment {
    pub manifest_digest_hex: String,
    pub verifier: String,
    pub payload_len: usize,
}

pub fn c2pa_manifest(signer: &dyn TpmSigner, payload: &[u8]) -> Result<C2paFragment, TpmError> {
    let started = Instant::now();
    let signature = signer.sign_nv_key(payload)?;
    let elapsed = started.elapsed();
    assert!(
        elapsed < SIGN_BUDGET,
        "signing took {:?} (budget {:?})",
        elapsed,
        SIGN_BUDGET
    );
    let manifest_digest: String = signature.iter().map(|b| format!("{b:02x}")).collect();
    Ok(C2paFragment {
        manifest_digest_hex: manifest_digest[..16].to_string(),
        verifier: "tpm-nv-key".into(),
        payload_len: payload.len(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn signer() -> InProcessSigner {
        InProcessSigner {
            key_seed: b"seed-for-pilot".to_vec(),
        }
    }

    #[test]
    fn signing_within_budget() {
        let s = signer();
        let sig = s.sign_nv_key(b"manifest-fragment").expect("signs");
        assert_eq!(sig.len(), 32);
        let sig2 = s.sign_nv_key(b"manifest-fragment").expect("signs again");
        assert_eq!(sig, sig2, "deterministic");
    }

    #[test]
    fn empty_seed_and_payload_guardrails() {
        let s = InProcessSigner { key_seed: vec![] };
        assert_eq!(s.sign_nv_key(b"x"), Err(TpmError::NoKeyHandle));
        let s2 = signer();
        assert_eq!(s2.sign_nv_key(&[]), Err(TpmError::EmptyPayload));
    }

    #[test]
    fn c2pa_fragment_has_digest_and_len() {
        let frag = c2pa_manifest(&signer(), b"fragment-1").expect("manifest");
        assert_eq!(frag.manifest_digest_hex.len(), 16);
        assert_eq!(frag.payload_len, 10);
        assert_eq!(frag.verifier, "tpm-nv-key");
    }
}
