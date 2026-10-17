//! Payload guardrails (docs/09 §6): JPEG ≤ 640² or ≤ 512 KB per image,
//! CSI feature vectors ≤ 4 KB — raw CSI binaries never leave the edge.

use std::io::Cursor;

use crate::error::VlmError;

pub const DEFAULT_MAX_IMAGE_SIDE: u32 = 640;
pub const DEFAULT_MAX_IMAGE_BYTES: usize = 512 * 1024;
pub const DEFAULT_MAX_VECTOR_BYTES: usize = 4 * 1024;
pub const DEFAULT_MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct PayloadCaps {
    pub max_image_side: u32,
    pub max_image_bytes: usize,
    pub max_vector_bytes: usize,
    pub max_prompt_bytes: usize,
}

impl Default for PayloadCaps {
    fn default() -> Self {
        Self {
            max_image_side: DEFAULT_MAX_IMAGE_SIDE,
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
            max_vector_bytes: DEFAULT_MAX_VECTOR_BYTES,
            max_prompt_bytes: DEFAULT_MAX_PROMPT_BYTES,
        }
    }
}

/// Header-only decode of JPEG dimensions via the `image` crate.
pub fn jpeg_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

impl PayloadCaps {
    /// Sizing gate for a frame the orchestrator is about to ship upstream.
    /// A JPEG that fails these caps is rejected *before* any provider call.
    pub fn check_image_jpeg(&self, bytes: &[u8]) -> Result<(u32, u32), VlmError> {
        if bytes.len() > self.max_image_bytes {
            return Err(VlmError::InvalidPayload(format!(
                "jpeg {} bytes exceeds cap {}",
                bytes.len(),
                self.max_image_bytes
            )));
        }
        let (w, h) = jpeg_dims(bytes)
            .ok_or_else(|| VlmError::InvalidPayload("not a decodable jpeg header".to_string()))?;
        if w.max(h) > self.max_image_side {
            return Err(VlmError::InvalidPayload(format!(
                "jpeg {w}x{h} exceeds side cap {}",
                self.max_image_side
            )));
        }
        Ok((w, h))
    }

    /// CSI arrives upstream as feature vectors only (docs/09 §6), never raw.
    pub fn check_feature_vector(&self, bytes: &[u8]) -> Result<(), VlmError> {
        if bytes.len() > self.max_vector_bytes {
            return Err(VlmError::InvalidPayload(format!(
                "feature vector {} bytes exceeds cap {}",
                bytes.len(),
                self.max_vector_bytes
            )));
        }
        Ok(())
    }
}

/// base64-encode for the `*_b64` prompt fields.
pub fn encode_b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn valid_small_jpeg_passes_and_reports_dims() {
        let bytes = testutil::jpeg_bytes(320, 240);
        assert_eq!(jpeg_dims(&bytes), Some((320, 240)));
        let caps = PayloadCaps::default();
        assert_eq!(caps.check_image_jpeg(&bytes).unwrap(), (320, 240));
    }

    #[test]
    fn oversized_side_is_rejected() {
        let bytes = testutil::jpeg_bytes(800, 400);
        let err = PayloadCaps::default().check_image_jpeg(&bytes).unwrap_err();
        assert!(matches!(err, VlmError::InvalidPayload(m) if m.contains("exceeds side cap 640")));
    }

    #[test]
    fn oversized_bytes_are_rejected_before_decode() {
        let bytes = testutil::jpeg_bytes(320, 240);
        let caps = PayloadCaps {
            max_image_bytes: 10,
            ..PayloadCaps::default()
        };
        let err = caps.check_image_jpeg(&bytes).unwrap_err();
        assert!(matches!(err, VlmError::InvalidPayload(m) if m.contains("exceeds cap 10")));
    }

    #[test]
    fn non_jpeg_payload_is_rejected() {
        let caps = PayloadCaps::default();
        assert!(caps.check_image_jpeg(b"definitely not a jpeg").is_err());
    }

    #[test]
    fn feature_vector_cap_is_enforced() {
        let caps = PayloadCaps::default();
        caps.check_feature_vector(&[0u8; 4096]).unwrap();
        let err = caps.check_feature_vector(&[0u8; 4097]).unwrap_err();
        assert!(matches!(err, VlmError::InvalidPayload(_)));
    }

    #[test]
    fn b64_roundtrip() {
        let bytes = testutil::jpeg_bytes(64, 64);
        let encoded = encode_b64(&bytes);
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded)
            .map_err(|e| e.to_string())
            .unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(decoded, bytes);
    }
}
