//! File-backed fixture source (docs/04 §7, docs/14 §3): replays a directory
//! of PNG frames in lexicographic order at a configured fps with fixed
//! config-driven timestamps (no wall-clock reads).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use common_types::{CameraId, EdgeStamp, FrameView, PixelFormat};

use crate::error::CctvError;
use crate::source::{SourceAdapter, SourceProps};

pub struct FileFrameSource {
    camera_id: CameraId,
    frames: Vec<PathBuf>,
    cursor: usize,
    fps: f32,
    start: DateTime<Utc>,
    props: Option<SourceProps>,
}

impl FileFrameSource {
    pub fn open(
        dir: &Path,
        camera_id: CameraId,
        fps: f32,
        start: DateTime<Utc>,
    ) -> Result<Self, CctvError> {
        let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| CctvError::CorruptFixture {
                path: dir.to_path_buf(),
                reason: e.to_string(),
            })?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            })
            .collect();
        frames.sort();
        if frames.is_empty() {
            return Err(CctvError::CorruptFixture {
                path: dir.to_path_buf(),
                reason: "no .png frames found".to_string(),
            });
        }
        if !(fps.is_finite() && fps > 0.0) {
            return Err(CctvError::CorruptFixture {
                path: dir.to_path_buf(),
                reason: "fps must be positive".to_string(),
            });
        }
        Ok(Self {
            camera_id,
            frames,
            cursor: 0,
            fps,
            start,
            props: None,
        })
    }

    fn decode(&mut self, path: &Path) -> Result<FrameView, CctvError> {
        let img = image::open(path).map_err(|e| CctvError::CorruptFixture {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let rgb = img.to_rgb8();
        let (width, height) = (img.width(), img.height());
        let mut data = Vec::with_capacity(rgb.width() as usize * rgb.height() as usize * 3);
        for px in rgb.pixels() {
            data.push(px[2]);
            data.push(px[1]);
            data.push(px[0]);
        }
        let dt = self.start
            + chrono::Duration::milliseconds(
                (f64::from(self.cursor as u32) * (1000.0 / f64::from(self.fps))).round() as i64,
            );
        Ok(FrameView {
            camera_id: self.camera_id.clone(),
            ts: EdgeStamp::new(dt, u64::from(self.cursor as u32), 0, true),
            width,
            height,
            format: PixelFormat::Bgr8,
            data,
        })
    }
}

#[async_trait]
impl SourceAdapter for FileFrameSource {
    async fn next_frame(&mut self) -> Option<Result<FrameView, CctvError>> {
        if self.cursor >= self.frames.len() {
            return None;
        }
        let path = self.frames[self.cursor].clone();
        let decoded = self.decode(&path);
        if decoded.is_ok() {
            if self.props.is_none() {
                let view = decoded.as_ref().ok();
                if let Some(view) = view {
                    self.props = Some(SourceProps {
                        fps: self.fps,
                        width: view.width,
                        height: view.height,
                    });
                }
            }
            self.cursor += 1;
        }
        Some(decoded)
    }

    fn props(&self) -> SourceProps {
        self.props.unwrap_or(SourceProps {
            fps: self.fps,
            width: 64,
            height: 36,
        })
    }
}

/// Timestamp helper for fixture configs (UTC only).
pub fn fixture_start(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, hh, mm, ss)
        .earliest()
        .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::io::Write;

    fn write_png(dir: &Path, name: &str, gray: u8, w: u32, h: u32) -> PathBuf {
        let img = image::GrayImage::from_pixel(w, h, image::Luma([gray]));
        let path = dir.join(name);
        let _ = img.save(&path);
        path
    }

    #[tokio::test]
    async fn replays_sorted_frames_with_monotonic_pts() {
        let dir = tempfile::tempdir().unwrap();
        write_png(dir.path(), "b.png", 20, 64, 36);
        write_png(dir.path(), "a.png", 250, 64, 36);

        let camera = CameraId::try_new("line2_cam1").unwrap();
        let start = fixture_start(2026, 10, 17, 6, 30, 0);
        let mut src = FileFrameSource::open(dir.path(), camera, 30.0, start).unwrap();

        let f0 = src.next_frame().await.unwrap().unwrap();
        let f1 = src.next_frame().await.unwrap().unwrap();
        assert_eq!(f0.data[0], 250, "lexicographic 'a' first");
        assert_eq!(src.props().width, 64);
        assert_eq!(src.props().fps, 30.0);
        assert!(
            (f1.ts.utc - f0.ts.utc).num_milliseconds() as f64 >= 1000.0 / 30.0 - 1.0,
            "pts spacing ~= 1/30s"
        );
        assert_eq!(f1.ts.stream_pts, 1);
        assert!(f1.ts.clock_ok);

        let _ = src.next_frame().await;
        assert!(
            src.next_frame().await.is_none(),
            "exhausted source ends orderly"
        );
    }

    #[test]
    fn empty_directory_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let camera = CameraId::try_new("cam").unwrap();
        assert!(
            FileFrameSource::open(dir.path(), camera, 30.0, fixture_start(2026, 1, 1, 0, 0, 0))
                .is_err()
        );
    }

    #[tokio::test]
    async fn corrupt_png_maps_to_fixture_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"not a png").unwrap();
        drop(f);
        let camera = CameraId::try_new("cam").unwrap();
        let mut src =
            FileFrameSource::open(dir.path(), camera, 30.0, fixture_start(2026, 1, 1, 0, 0, 0))
                .unwrap();
        match src.next_frame().await {
            Some(Err(CctvError::CorruptFixture { .. })) => {}
            other => panic!("expected CorruptFixture, got {other:?}"),
        }
    }
}
