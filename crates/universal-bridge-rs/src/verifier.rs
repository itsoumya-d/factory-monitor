//! Deep camera verification (oxvif-style): discovery to image sanity to
//! replay probes — a clone-method capture for fast offline prototyping.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerifyError {
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("snapshot payload is not a JPEG (magic bytes {0:02X?})")]
    NotJpeg(Vec<u8>),
    #[error("snapshot payload is empty")]
    EmptySnapshot,
    #[error("returned document looks like HTML ({0:?})")]
    HtmlMasquerade(Vec<String>),
    #[error("device info missing required key {0:?}")]
    DeviceInfoKeyMissing(String),
    #[error("profile G replay endpoint refused")]
    ReplayProbeFailed,
}

/// Minimal transport the verifier drives; real oxvif client plugs in here.
pub trait OnvifTransport: Send + Sync {
    fn probe(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, VerifyError>;
}

pub struct DeepVerifier<'t> {
    pub transport: &'t dyn OnvifTransport,
}

impl<'t> DeepVerifier<'t> {
    pub fn new(transport: &'t dyn OnvifTransport) -> Self {
        Self { transport }
    }

    /// Device info: required keys per docs (firmware, model, serial).
    pub fn device_info(&self) -> Result<DeviceInfo, VerifyError> {
        let body = self
            .transport
            .probe("/onvif/device_service/GetDeviceInformation", &[])?;
        let info = DeviceInfo::parse(&body)?;
        info.validate()?;
        Ok(info)
    }

    /// Snapshot must be a real JPEG (magic FFD8FF), never 0 bytes, and
    /// never an HTML document wrapped in HTTP 200.
    pub fn snapshot_is_real(&self) -> Result<(), VerifyError> {
        let body = self
            .transport
            .probe("/onvif/media_service/GetSnapshotUri", &[])?;
        if body.is_empty() {
            return Err(VerifyError::EmptySnapshot);
        }
        if body.len() >= 3 && !(body[0] == 0xFF && body[1] == 0xD8 && body[2] == 0xFF) {
            let text: Vec<String> = String::from_utf8_lossy(&body)
                .lines()
                .take(2)
                .map(str::to_string)
                .collect();
            if text
                .iter()
                .any(|l| l.contains("<!DOCTYPE") || l.contains("<html"))
            {
                return Err(VerifyError::HtmlMasquerade(text));
            }
            return Err(VerifyError::NotJpeg(body));
        }
        Ok(())
    }

    /// Profile G replay probe: the endpoint must accept a replay window
    /// request without failing.
    pub fn replay_probe(&self) -> Result<(), VerifyError> {
        self.transport
            .probe("/onvif/recording_service/GetReplayUri", b"replay-window")
            .map(|_| ())
    }

    pub fn verify_all(&self) -> Result<DeviceInfo, VerifyError> {
        let info = self.device_info()?;
        self.snapshot_is_real()?;
        self.replay_probe()?;
        Ok(info)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub firmware: String,
    pub model: String,
    pub serial: String,
}

impl DeviceInfo {
    fn parse(body: &[u8]) -> Result<Self, VerifyError> {
        let text = String::from_utf8_lossy(body);
        let field = |key: &str| -> Option<String> {
            text.lines()
                .find(|l| l.contains(key))
                .map(|l| l.trim().to_string())
        };
        Ok(Self {
            firmware: field("firmware")
                .ok_or_else(|| VerifyError::DeviceInfoKeyMissing("firmware".into()))?,
            model: field("model")
                .ok_or_else(|| VerifyError::DeviceInfoKeyMissing("model".into()))?,
            serial: field("serial")
                .ok_or_else(|| VerifyError::DeviceInfoKeyMissing("serial".into()))?,
        })
    }

    fn validate(&self) -> Result<(), VerifyError> {
        if self.firmware.trim().is_empty()
            || self.model.trim().is_empty()
            || self.serial.trim().is_empty()
        {
            return Err(VerifyError::DeviceInfoKeyMissing(
                "one of the required keys".into(),
            ));
        }
        Ok(())
    }
}

/// Camera-clone recorder — captures the transcript of method calls so a
/// pilot site can be prototyped offline without live hardware.
#[derive(Debug, Clone, PartialEq)]
pub struct CloneTranscript {
    pub calls: Vec<String>,
}

impl CloneTranscript {
    #[must_use]
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }

    pub fn record(&mut self, call: String) {
        self.calls.push(call);
    }
}

impl Default for CloneTranscript {
    fn default() -> Self {
        Self::new()
    }
}

/// The recorder needs mutation; the mirror transport (separate type)
/// records into an owned transcript for testing/prototypes.
pub struct MockTransport {
    pub responses: std::collections::BTreeMap<String, Vec<u8>>,
    pub transcript: std::sync::Mutex<CloneTranscript>,
}

impl MockTransport {
    pub fn with_camera_valid() -> Self {
        let mut m = Self {
            responses: Default::default(),
            transcript: std::sync::Mutex::new(CloneTranscript::new()),
        };
        m.responses.insert(
            "/onvif/device_service/GetDeviceInformation".to_string(),
            b"firmware=4.7.1\nmodel=X1\nserial=SN123".to_vec(),
        );
        m.responses.insert(
            "/onvif/media_service/GetSnapshotUri".to_string(),
            vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xD9],
        );
        m.responses.insert(
            "/onvif/recording_service/GetReplayUri".to_string(),
            b"ok".to_vec(),
        );
        m
    }

    pub fn with_html_fakes() -> Self {
        let mut m = Self {
            responses: Default::default(),
            transcript: std::sync::Mutex::new(CloneTranscript::new()),
        };
        m.responses.insert(
            "/onvif/media_service/GetSnapshotUri".to_string(),
            b"<!DOCTYPE html><html><body>camera login</body></html>".to_vec(),
        );
        m
    }
}

impl OnvifTransport for MockTransport {
    fn probe(&self, path: &str, _body: &[u8]) -> Result<Vec<u8>, VerifyError> {
        self.transcript
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(format!("GET {path}"));
        self.responses
            .get(path)
            .cloned()
            .ok_or_else(|| VerifyError::Transport(format!("no mock response for {path}")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn happy_camera_passes_all_gates() {
        let device = MockTransport::with_camera_valid();
        let verifier = DeepVerifier::new(&device);
        verifier.verify_all().expect("up-to-spec camera");
    }

    #[test]
    fn html_fakes_fail_snapshot_check() {
        let device = MockTransport::with_html_fakes();
        let verifier = DeepVerifier::new(&device);
        let got = verifier.snapshot_is_real();
        assert!(
            matches!(got, Err(VerifyError::HtmlMasquerade(_))),
            "got {got:?}"
        );
    }

    #[test]
    fn zero_byte_snapshot_fails() {
        let mut device = MockTransport::with_camera_valid();
        device
            .responses
            .insert("/onvif/media_service/GetSnapshotUri".into(), vec![]);
        let verifier = DeepVerifier::new(&device);
        assert_eq!(verifier.snapshot_is_real(), Err(VerifyError::EmptySnapshot));
    }

    #[test]
    fn missing_device_keys_fail() {
        let mut device = MockTransport::with_camera_valid();
        device.responses.insert(
            "/onvif/device_service/GetDeviceInformation".into(),
            b"model=X1".to_vec(),
        );
        let verifier = DeepVerifier::new(&device);
        assert!(matches!(
            verifier.device_info(),
            Err(VerifyError::DeviceInfoKeyMissing(_))
        ));
    }

    #[test]
    fn replay_probe_documented() {
        let device = MockTransport::with_camera_valid();
        assert!(DeepVerifier::new(&device).replay_probe().is_ok());
    }

    #[test]
    fn mock_transcript_records_calls() {
        let device = MockTransport::with_camera_valid();
        let _ = DeepVerifier::new(&device).verify_all();
        assert!(
            device
                .transcript
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .calls
                .len()
                >= 3,
            "three probes recorded"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod transcript_tests {
    use super::CloneTranscript;

    #[test]
    fn transcript_records_in_call_order_and_clones() {
        let mut t = CloneTranscript::new();
        t.record("a".to_string());
        t.record("b".to_string());
        assert_eq!(t.calls, vec!["a".to_string(), "b".to_string()]);
        let snap = t.clone();
        assert_eq!(snap.calls, t.calls);
    }
}
