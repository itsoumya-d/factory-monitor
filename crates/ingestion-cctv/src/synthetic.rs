//! Synthetic frame source with deterministic geometry (docs/04 §7).
//!
//! Generates Gray8 frames: a base fill (optionally ramping to emulate the
//! lights-on/lights-off sweep) plus a moving bright belt-rect at a fixed
//! belt speed. Used by luminance, quorum, and ingestor tests — and as the
//! decode-path canary for the burn-in rig.

use async_trait::async_trait;
use common_types::{CameraId, EdgeStamp, FrameView, PixelFormat};

use crate::source::{SourceAdapter, SourceProps};

#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticScene {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub base_lux: f32,
    /// Optional linear ramp of `base_lux` over `ramp_steps` frames (dawn).
    pub ramp_to_lux: Option<f32>,
    pub ramp_steps: u64,
    pub belt_speed_px: f32,
    pub total_frames: u64,
}

impl Default for SyntheticScene {
    fn default() -> Self {
        Self {
            width: 64,
            height: 36,
            fps: 30.0,
            base_lux: 4.0,
            ramp_to_lux: None,
            ramp_steps: 0,
            belt_speed_px: 2.0,
            total_frames: 120,
        }
    }
}

pub struct SyntheticFrameSource {
    camera_id: CameraId,
    scene: SyntheticScene,
    cursor: u64,
    step_size: f32,
}

impl SyntheticFrameSource {
    pub fn new(camera_id: CameraId, scene: SyntheticScene) -> Self {
        let step_size = match scene.ramp_to_lux {
            Some(target) if scene.ramp_steps > 0 => {
                (target - scene.base_lux) / scene.ramp_steps as f32
            }
            _ => 0.0,
        };
        Self {
            camera_id,
            scene,
            cursor: 0,
            step_size,
        }
    }

    fn render(&self, frame_idx: u64) -> FrameView {
        let w = self.scene.width;
        let h = self.scene.height;
        let mut data = vec![0u8; (w * h) as usize];
        let base = (self.scene.base_lux
            + self.step_size * (frame_idx as f32).min(self.scene.ramp_steps as f32))
        .clamp(0.0, 255.0);
        data.fill(base as u8);

        let rect_w = 10u32;
        let span = w - rect_w;
        let rect_x =
            ((frame_idx as f64 * f64::from(self.scene.belt_speed_px)) % f64::from(span)) as u32;
        for y in (h / 4).min(h)..((h / 4) + 8).min(h) {
            for x in rect_x.min(w)..(rect_x + rect_w).min(w) {
                data[(y * w + x) as usize] = 255;
            }
        }

        let dt_ms = (1_000.0 / f64::from(self.scene.fps)).round() as i64;
        let ts = EdgeStamp::new(
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
                + chrono::Duration::milliseconds(dt_ms * frame_idx as i64),
            frame_idx,
            0,
            true,
        );
        FrameView {
            camera_id: self.camera_id.clone(),
            ts,
            width: w,
            height: h,
            format: PixelFormat::Gray8,
            data,
        }
    }
}

#[async_trait]
impl SourceAdapter for SyntheticFrameSource {
    async fn next_frame(
        &mut self,
    ) -> Option<Result<common_types::FrameView, crate::error::CctvError>> {
        if self.cursor >= self.scene.total_frames {
            return None;
        }
        let view = self.render(self.cursor);
        self.cursor += 1;
        Some(Ok(view))
    }

    fn props(&self) -> SourceProps {
        SourceProps {
            fps: self.scene.fps,
            width: self.scene.width,
            height: self.scene.height,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[tokio::test]
    async fn belt_moves_and_frames_count_matches() {
        let scene = SyntheticScene {
            total_frames: 12,
            ..Default::default()
        };
        let camera = CameraId::try_new("synth").unwrap();
        let mut src = SyntheticFrameSource::new(camera, scene);
        let f0 = src.next_frame().await.unwrap().unwrap();
        let f1 = src.next_frame().await.unwrap().unwrap();
        assert_eq!(f0.ts.stream_pts, 0);
        assert_eq!(f1.ts.stream_pts, 1);
        assert_eq!(f1.format, PixelFormat::Gray8);
        let belt_row = f0.height / 4;
        assert_ne!(
            f0.data[(belt_row * f0.width) as usize],
            f1.data[(belt_row * f1.width) as usize],
            "belt motion changes the start of the belt row"
        );
        for _ in 0..14 {
            let _ = src.next_frame().await;
        }
        assert!(src.next_frame().await.is_none(), "honors total_frames");
    }

    #[tokio::test]
    async fn luminance_ramp_rises_monotonically() {
        let scene = SyntheticScene {
            total_frames: 60,
            base_lux: 0.0,
            ramp_to_lux: Some(120.0),
            ramp_steps: 60,
            ..Default::default()
        };
        let camera = CameraId::try_new("synth").unwrap();
        let mut src = SyntheticFrameSource::new(camera, scene);
        let f0 = src.next_frame().await.unwrap().unwrap();
        let mut last = f0.data[0] as f32;
        for _ in 1..40 {
            let f = src.next_frame().await.unwrap().unwrap();
            let cent = f.data[f.data.len() / 2] as i32;
            let base_fill = if cent == 255 { last } else { cent as f32 };
            assert!(
                base_fill >= last - 1.0,
                "base fill must not decrease while ramping"
            );
            last = base_fill;
        }
    }
}
