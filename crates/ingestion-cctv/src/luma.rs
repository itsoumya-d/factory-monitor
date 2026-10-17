//! BT.709 luma tooling for the lights-on trigger (docs/04 §2).
//!
//! Documents a single proxy definition: `lux_proxy = mean(64x36 box
//! downsample, BT.709 luma) * calibration_gain`. Gains are bench-measured per
//! camera with a lux meter and stored in the camera config (docs/04 §6).

use common_types::{FrameView, PixelFormat};

use crate::error::CctvError;

pub const DOWNSAMPLE_W: u32 = 64;
pub const DOWNSAMPLE_H: u32 = 36;

/// Per-pixel luma row buffer length constant for a single row pass.
const BGR_CHANNELS: usize = 3;
const BGRX_CHANNELS: usize = 4;

/// Extract a `width x height` single-plane luma buffer for supported formats.
///
/// * `Bgr8`   — bytes `[B, G, R]` per pixel; proxy = BT.709 weights.
/// * `Bgrx8`  — bytes `[B, G, R, X]` per pixel.
/// * `Gray8`  — plane passthrough.
/// * `Nv12`   — luma plane is the first `width*height` bytes.
pub fn luma_plane(frame: &FrameView) -> Result<Vec<u8>, CctvError> {
    if frame.width < DOWNSAMPLE_W || frame.height < DOWNSAMPLE_H {
        return Err(CctvError::FrameTooSmall {
            width: frame.width,
            height: frame.height,
        });
    }
    let pixels = (frame.width * frame.height) as usize;
    let expected = match frame.format {
        PixelFormat::Bgr8 => pixels * BGR_CHANNELS,
        PixelFormat::Bgrx8 => pixels * BGRX_CHANNELS,
        PixelFormat::Gray8 => pixels,
        // NV12 carries luma (w*h) + interleaved chroma (w*h/2) = 1.5 bytes/px.
        PixelFormat::Nv12 => pixels + pixels / 2,
    };
    if frame.data.len() != expected {
        return Err(CctvError::CorruptFixture {
            path: std::path::PathBuf::from(format!(
                "{}:{}x{}:{:?}",
                frame.camera_id, frame.width, frame.height, frame.format
            )),
            reason: format!(
                "declared geometry needs {} bytes but frame carries {}",
                expected,
                frame.data.len()
            ),
        });
    }
    Ok(match frame.format {
        PixelFormat::Gray8 => frame.data.clone(),
        PixelFormat::Nv12 => frame.data[..pixels].to_vec(),
        PixelFormat::Bgr8 => {
            let mut out = vec![0u8; pixels];
            for (chunk, out) in frame.data.chunks_exact(BGR_CHANNELS).zip(out.iter_mut()) {
                *out = bgr_byte_to_luma(chunk);
            }
            out
        }
        PixelFormat::Bgrx8 => {
            let mut out = vec![0u8; pixels];
            for (chunk, out) in frame.data.chunks_exact(BGRX_CHANNELS).zip(out.iter_mut()) {
                *out = bgr_byte_to_luma(&chunk[..BGR_CHANNELS]);
            }
            out
        }
    })
}

#[inline]
fn bgr_byte_to_luma(bgr: &[u8]) -> u8 {
    let b = f32::from(bgr[0]);
    let g = f32::from(bgr[1]);
    let r = f32::from(bgr[2]);
    let v = 0.212_6 * r + 0.715_2 * g + 0.072_2 * b;
    v.round().clamp(0.0, 255.0) as u8
}

/// Box-average a luma plane into an exact `64x36` matrix (row-major).
///
/// Edge cells receive fractional weights so down- and (if ever) up-sizing both
/// yield means over stable support; the function never divides by zero.
pub fn downsample_to_64x36(luma: &[u8], width: u32, height: u32) -> Result<Vec<f32>, CctvError> {
    if (width as usize) * (height as usize) != luma.len() {
        return Err(CctvError::CorruptFixture {
            path: std::path::PathBuf::from("downsample"),
            reason: "luma buffer length does not match width*height".to_string(),
        });
    }
    let w = f64::from(width);
    let h = f64::from(height);
    let mut out = vec![0f32; (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize];
    for ty in 0..DOWNSAMPLE_H {
        let y0 = f64::from(ty) * h / f64::from(DOWNSAMPLE_H);
        let y1 = f64::from(ty + 1) * h / f64::from(DOWNSAMPLE_H);
        for tx in 0..DOWNSAMPLE_W {
            let x0 = f64::from(tx) * w / f64::from(DOWNSAMPLE_W);
            let x1 = f64::from(tx + 1) * w / f64::from(DOWNSAMPLE_W);
            let mut sum = 0f64;
            // Support covers every overlapping pixel row/col; the `weight`
            // helper applies fractional coverage at the edges so the summed
            // weight of a cell is exactly its area (uniform fill invariant).
            for y in y0 as usize
                ..(y1.ceil() as usize)
                    .min(height as usize)
                    .max(y0 as usize + 1)
            {
                let wy = weight(y0, y1, y as f64);
                for x in x0 as usize
                    ..(x1.ceil() as usize)
                        .min(width as usize)
                        .max(x0 as usize + 1)
                {
                    let wx = weight(x0, x1, x as f64);
                    let sample = f64::from(luma[(y * width as usize) + x]);
                    sum += sample * wx * wy;
                }
            }
            let area = (y1 - y0) * (x1 - x0);
            let idx = (ty * DOWNSAMPLE_W + tx) as usize;
            out[idx] = if area > 0.0 { (sum / area) as f32 } else { 0.0 };
        }
    }
    Ok(out)
}

#[inline]
fn weight(a0: f64, a1: f64, i: f64) -> f64 {
    (a1.min(i + 1.0) - a0.max(i)).clamp(0.0, 1.0)
}

/// Camera gain applied to the proxy luma -> "lux proxy" consumed by the
/// trigger and the `FactoryEnvState.lux_proxy` field (docs/06 §1).
pub fn lux_proxy(frame: &FrameView, gain: f32) -> Result<f32, CctvError> {
    if !(gain.is_finite() && gain > 0.0) {
        return Err(CctvError::InvalidLumaGain(frame.camera_id.to_string()));
    }
    let plane = luma_plane(frame)?;
    let ds = downsample_to_64x36(&plane, frame.width, frame.height)?;
    Ok((ds.iter().sum::<f32>() / ds.len() as f32) * gain)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use common_types::{CameraId, EdgeStamp, PixelFormat};
    use proptest::prelude::*;

    fn frame(
        camera: &str,
        width: u32,
        height: u32,
        format: PixelFormat,
        data: Vec<u8>,
    ) -> FrameView {
        FrameView {
            camera_id: CameraId::try_new(camera).unwrap(),
            ts: EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, 0, 0, true),
            width,
            height,
            format,
            data,
        }
    }

    #[test]
    fn gray_white_and_black_proxies() {
        let white = frame(
            "cam",
            DOWNSAMPLE_W,
            DOWNSAMPLE_H,
            PixelFormat::Gray8,
            vec![255u8; (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize],
        );
        assert!((lux_proxy(&white, 1.0).unwrap() - 255.0).abs() < 1e-3);
        let dark = frame(
            "cam",
            DOWNSAMPLE_W,
            DOWNSAMPLE_H,
            PixelFormat::Gray8,
            vec![0u8; (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize],
        );
        assert!((lux_proxy(&dark, 1.0).unwrap()).abs() < 1e-3);
    }

    #[test]
    fn bgr_weighting_matches_bt709() {
        let pure_red = frame(
            "cam",
            DOWNSAMPLE_W,
            DOWNSAMPLE_H,
            PixelFormat::Bgr8,
            [0, 0, 255].repeat((DOWNSAMPLE_W * DOWNSAMPLE_H) as usize),
        );
        let v = lux_proxy(&pure_red, 1.0).unwrap();
        assert!(((0.212_6 * 255.0) - v).abs() < 1.0);
    }

    #[test]
    fn nv12_uses_luma_plane_only() {
        let luma = vec![128u8; (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize];
        let chroma = vec![255u8; (DOWNSAMPLE_W * DOWNSAMPLE_H / 2) as usize];
        let mut data = luma.clone();
        data.extend(chroma);
        let view = frame("cam", DOWNSAMPLE_W, DOWNSAMPLE_H, PixelFormat::Nv12, data);
        assert!((lux_proxy(&view, 1.0).unwrap() - 128.0).abs() < 1.0);
    }

    #[test]
    fn wrong_len_rejected() {
        let small = frame(
            "cam",
            DOWNSAMPLE_W,
            DOWNSAMPLE_H,
            PixelFormat::Gray8,
            vec![0; 10],
        );
        assert!(lux_proxy(&small, 1.0).is_err());
    }

    #[test]
    fn downsample_of_uniform_is_uniform() {
        let plane = vec![77u8; (128 * 72) as usize];
        let ds = downsample_to_64x36(&plane, 128, 72).unwrap();
        assert!(ds.iter().all(|v| (v - 77.0).abs() < 1e-3));
    }

    proptest! {
        #[test]
        fn downsample_conservation_sum_bound(
            width in 64u32..320,
            height in 36u32..243,
            fill in 0u8..=255,
        ) {
            let plane = vec![fill; (width * height) as usize];
            let ds = downsample_to_64x36(&plane, width, height).unwrap();
            prop_assert_eq!(ds.len(), (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize);
            for v in ds {
                prop_assert!(((f64::from(fill)) - v as f64).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn gain_must_be_positive_and_finite() {
        let base = frame(
            "cam",
            DOWNSAMPLE_W,
            DOWNSAMPLE_H,
            PixelFormat::Gray8,
            vec![10; (DOWNSAMPLE_W * DOWNSAMPLE_H) as usize],
        );
        assert!(lux_proxy(&base, 0.0).is_err());
        assert!(lux_proxy(&base, f32::NAN).is_err());
        assert!(lux_proxy(&base, 2.0).is_ok());
    }
}
