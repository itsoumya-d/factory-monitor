//! Scripted [`VisionDetector`] for CI and wiring tests (docs/14 §2: sources
//! and models are faked in CI). Consumes a deterministic screenplay of
//! detections frame by frame.

use std::collections::VecDeque;

use async_trait::async_trait;

use common_types::bounds::DetectorThresholds;
use common_types::frame::FrameView;
use common_types::ids::ModelVersion;
use common_types::traits::{DetectError, DetectOutput, VisionDetector};

/// Feed once-per-frame scripted detections. Exhausted screenplays yield
/// empty frames; detection requires a prior successful `warm()` call.
pub struct ScriptedVisionDetector {
    version: ModelVersion,
    script: VecDeque<Vec<common_types::traits::Detection>>,
    warm: bool,
}

impl ScriptedVisionDetector {
    pub fn new(version: ModelVersion, script: Vec<Vec<common_types::traits::Detection>>) -> Self {
        Self {
            version,
            script: script.into(),
            warm: false,
        }
    }

    pub fn remaining(&self) -> usize {
        self.script.len()
    }
}

#[async_trait]
impl VisionDetector for ScriptedVisionDetector {
    fn version(&self) -> ModelVersion {
        self.version.clone()
    }

    async fn detect(
        &mut self,
        _frame: &FrameView,
        _thresholds: DetectorThresholds,
    ) -> Result<DetectOutput, DetectError> {
        if !self.warm {
            return Err(DetectError::NotWarmed);
        }
        let detections = self.script.pop_front().unwrap_or_default();
        Ok(DetectOutput {
            detections,
            elapsed_ms: 0.0,
        })
    }

    async fn warm(&mut self) -> Result<(), DetectError> {
        self.warm = true;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use common_types::edge_stamp::EdgeStamp;
    use common_types::ids::CameraId;
    use common_types::traits::Detection;

    fn frame() -> FrameView {
        FrameView {
            camera_id: CameraId::try_new("cam").unwrap(),
            ts: EdgeStamp {
                utc: Utc::now(),
                stream_pts: 0,
                ptp_offset_ns: 0,
                clock_ok: true,
            },
            width: 4,
            height: 4,
            format: common_types::frame::PixelFormat::Bgr8,
            data: vec![0; 4 * 4 * 3],
        }
    }

    fn det(cx: f32) -> Detection {
        Detection {
            bbox_xyxy: [cx, 0.0, cx + 10.0, 10.0],
            score: 0.9,
            class_id: 0,
        }
    }

    fn thresholds() -> DetectorThresholds {
        DetectorThresholds {
            conf_threshold: 0.45,
            iou_threshold: 0.45,
            track_buffer_frames: 45,
        }
    }

    #[tokio::test]
    async fn detects_require_warm() {
        let mut detector = ScriptedVisionDetector::new(
            ModelVersion::try_new("mock-v1").unwrap(),
            vec![vec![det(0.0)], vec![det(10.0)]],
        );
        let result = detector.detect(&frame(), thresholds()).await;
        assert!(matches!(result, Err(DetectError::NotWarmed)));
    }

    #[tokio::test]
    async fn scripted_detections_in_order() {
        let mut detector = ScriptedVisionDetector::new(
            ModelVersion::try_new("mock-v1").unwrap(),
            vec![vec![det(0.0)], vec![det(10.0), det(20.0)]],
        );
        detector.warm().await.unwrap();
        assert_eq!(detector.remaining(), 2);
        assert_eq!(
            detector.version().as_str(),
            "mock-v1",
            "version flows through to the trait"
        );
        let first = detector.detect(&frame(), thresholds()).await.unwrap();
        assert_eq!(first.detections.len(), 1);
        assert_eq!(detector.remaining(), 1);
        let second = detector.detect(&frame(), thresholds()).await.unwrap();
        assert_eq!(second.detections.len(), 2);
        assert_eq!(detector.remaining(), 0);
        let third = detector.detect(&frame(), thresholds()).await.unwrap();
        assert!(third.detections.is_empty(), "exhausted script is silent");
    }
}
