//! Letterbox preprocessing (docs/07 §2): frame → normalized CHW f32 buffer
//! with aspect-preserving resize, RGB order, per-family normalization, and
//! the exact inverse map for detection boxes back to frame pixels.
//!
//! Hand-rolled on plain slices — no opencv in the hot path; the same layout
//! later feeds ONNX Runtime (`[1, 3, 640, 640]`).

use common_types::{CameraId, FrameView, PixelFormat};

use crate::error::InferenceError;

/// Normalization constants per model family (docs/07 §2 "constants per model
/// family"). All produce a CHW `f32` buffer with RGB channel order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    /// YOLO-style: `x/255`, RGB.
    Rgb01,
    /// RF-DETR-style ImageNet stats in u8 space: `(x - mean) / std`, RGB.
    ImageNet,
}

impl ModelFamily {
    const fn mean_u8(self) -> [f32; 3] {
        match self {
            Self::Rgb01 => [0.0; 3],
            Self::ImageNet => [124.0, 117.0, 104.0],
        }
    }

    const fn inv_std(self) -> [f32; 3] {
        match self {
            Self::Rgb01 => [1.0 / 255.0; 3],
            Self::ImageNet => [1.0 / 58.4, 1.0 / 57.1, 1.0 / 57.4],
        }
    }
}

pub const DEFAULT_TARGET: (u32, u32) = (640, 640);
/// YOLO letterbox padding gray (matches the reference preproc convention).
const PAD_GRAY_U8: f32 = 114.0;

/// Upper bound on source pixels so a corrupt capture can't OOM the edge box
/// (≈ 4K + slack). Larger frames are rejected with `FrameTooLarge`.
pub const MAX_SOURCE_PIXELS: u64 = 12_400_000;

/// Where the model-content rectangle lands inside the padded target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxPlan {
    pub target: (u32, u32),
    pub scale: f32,
    /// Centered content rect in target pixel coordinates.
    pub x0: u32,
    pub y0: u32,
    pub content_w: u32,
    pub content_h: u32,
}

impl LetterboxPlan {
    /// `None` only for degenerate zero-dimension inputs (targets must be
    /// ≥ 1×1; source must not be empty).
    pub fn compute(source: (u32, u32), target: (u32, u32)) -> Option<Self> {
        if source.0 == 0 || source.1 == 0 || target.0 == 0 || target.1 == 0 {
            return None;
        }
        let sw = source.0 as f32;
        let sh = source.1 as f32;
        let scale = ((target.0 as f32) / sw).min((target.1 as f32) / sh);
        let content_w = ((sw * scale).round() as u32).max(1).min(target.0);
        let content_h = ((sh * scale).round() as u32).max(1).min(target.1);
        Some(Self {
            target,
            scale,
            x0: (target.0 - content_w) / 2,
            y0: (target.1 - content_h) / 2,
            content_w,
            content_h,
        })
    }

    /// Map a detection box in padded-target pixel coords back to the source
    /// frame (docs/07 §2 "Decode boxes" consumers).
    pub fn unmap_xyxy(&self, box_xyxy: [f32; 4]) -> [f32; 4] {
        let [x1, y1, x2, y2] = box_xyxy;
        let to_src = |v: f32, pad: u32| (v - pad as f32) / self.scale;
        [
            to_src(x1, self.x0),
            to_src(y1, self.y0),
            to_src(x2, self.x0),
            to_src(y2, self.y0),
        ]
    }
}

/// Normalized CHW tensor ready for a detector session.
#[derive(Debug, PartialEq)]
pub struct NormalizedTensor {
    /// `[3][H][W]` row-major: channel plane, row, column.
    pub data: Vec<f32>,
    pub plan: LetterboxPlan,
}

impl NormalizedTensor {
    pub fn channel(&self, c: usize, y: usize, x: usize) -> f32 {
        let w = self.plan.target.0 as usize;
        let h = self.plan.target.1 as usize;
        self.data[c * h * w + y * w + x]
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

struct RgbPlanes {
    w: usize,
    h: usize,
    /// [R, G, B] planes, row-major.
    planes: [Vec<u8>; 3],
}

fn required_len(
    camera: &CameraId,
    fmt: PixelFormat,
    w: u32,
    h: u32,
    actual: usize,
) -> Result<usize, InferenceError> {
    let pixels = u64::from(w) * u64::from(h);
    let expected = match fmt {
        PixelFormat::Bgr8 => 3 * pixels,
        PixelFormat::Bgrx8 => 4 * pixels,
        // NV12 interleaves a half-resolution chroma plane (+pixels/2); plain
        // grayscale has no planar tail.
        PixelFormat::Gray8 => pixels,
        PixelFormat::Nv12 => pixels + pixels / 2,
    };
    let expected = usize::try_from(expected).map_err(|_| InferenceError::FrameTooLarge {
        camera: camera.clone(),
        width: w,
        height: h,
    })?;
    if actual != expected {
        return Err(InferenceError::InvalidFrameBuffer {
            camera: camera.clone(),
            expected,
            actual,
            width: w,
            height: h,
        });
    }
    Ok(expected)
}

fn planes_of(frame: &FrameView) -> Result<RgbPlanes, InferenceError> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let camera = frame.camera_id.clone();
    if w == 0 || h == 0 {
        return Err(InferenceError::EmptyFrame { camera });
    }
    let pixels = frame.width as u64 * frame.height as u64;
    if pixels > MAX_SOURCE_PIXELS {
        return Err(InferenceError::FrameTooLarge {
            camera,
            width: frame.width,
            height: frame.height,
        });
    }
    required_len(
        &camera,
        frame.format,
        frame.width,
        frame.height,
        frame.data.len(),
    )?;

    let planes = match frame.format {
        PixelFormat::Bgr8 => {
            let mut r = Vec::with_capacity(w * h);
            let mut g = Vec::with_capacity(w * h);
            let mut b = Vec::with_capacity(w * h);
            for px in frame.data.chunks_exact(3) {
                b.push(px[0]);
                g.push(px[1]);
                r.push(px[2]);
            }
            [r, g, b]
        }
        PixelFormat::Bgrx8 => {
            let mut r = Vec::with_capacity(w * h);
            let mut g = Vec::with_capacity(w * h);
            let mut b = Vec::with_capacity(w * h);
            for px in frame.data.chunks_exact(4) {
                b.push(px[0]);
                g.push(px[1]);
                r.push(px[2]);
            }
            [r, g, b]
        }
        PixelFormat::Gray8 | PixelFormat::Nv12 => {
            // Detector input takes the luma plane three times; chroma carries
            // no additional brightness content.
            let luma = frame.data[..w * h].to_vec();
            [luma.clone(), luma.clone(), luma]
        }
    };
    Ok(RgbPlanes { w, h, planes })
}

struct Sample {
    x0: usize,
    x1: usize,
    t: f32,
}

/// Bilinear resize with the align-center mapping (OpenCV INTER_LINEAR):
/// source coordinate = (out + 0.5) * (src / dst) - 0.5, clamped at borders.
fn bilinear(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let sx_step = sw as f32 / dw as f32;
    let sy_step = sh as f32 / dh as f32;
    let mut xs: Vec<Sample> = Vec::with_capacity(dw);
    for out_x in 0..dw {
        let fx = ((out_x as f32 + 0.5) * sx_step - 0.5).max(0.0);
        let x0 = (fx as usize).min(sw - 1);
        let x1 = (x0 + 1).min(sw - 1);
        let t = fx - x0 as f32;
        xs.push(Sample { x0, x1, t });
    }
    let mut out = Vec::with_capacity(dw * dh);
    for out_y in 0..dh {
        let fy = ((out_y as f32 + 0.5) * sy_step - 0.5).max(0.0);
        let y0 = (fy as usize).min(sh - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let ty = fy - y0 as f32;
        let row_a = &src[y0 * sw..(y0 + 1) * sw];
        let row_b = &src[y1 * sw..(y1 + 1) * sw];
        for s in &xs {
            let top = f32::from(row_a[s.x0]) * (1.0 - s.t) + f32::from(row_a[s.x1]) * s.t;
            let bottom = f32::from(row_b[s.x0]) * (1.0 - s.t) + f32::from(row_b[s.x1]) * s.t;
            out.push(top * (1.0 - ty) + bottom * ty);
        }
    }
    out
}

/// Full preprocess pipeline: validate → RGB → bilinear letterbox → gray pad →
/// normalize → CHW f32.
pub fn preprocess(
    frame: &FrameView,
    family: ModelFamily,
    target: (u32, u32),
) -> Result<NormalizedTensor, InferenceError> {
    let src = planes_of(frame)?;
    let plan = LetterboxPlan::compute((frame.width, frame.height), target).ok_or(
        InferenceError::EmptyFrame {
            camera: frame.camera_id.clone(),
        },
    )?;
    let means = family.mean_u8();
    let inv_stds = family.inv_std();

    let (dw, dh) = (plan.target.0 as usize, plan.target.1 as usize);
    let mut data = vec![0.0_f32; 3 * dw * dh];
    for (c, plane) in src.planes.iter().enumerate() {
        let resized = bilinear(
            plane,
            src.w,
            src.h,
            plan.content_w as usize,
            plan.content_h as usize,
        );
        let mean = means[c];
        let inv_std = inv_stds[c];
        // Family-specific normalized pad: fill gray first, then apply the
        // same affine as content pixels.
        let pad_norm = ((PAD_GRAY_U8 - mean) * inv_std).max(0.0);
        let mut row_cursor = 0;
        for out_y in 0..dh {
            let out_row = out_y * dw;
            if out_y < plan.y0 as usize || out_y >= plan.y0 as usize + plan.content_h as usize {
                for x in 0..dw {
                    data[c * dw * dh + out_row + x] = pad_norm;
                }
                continue;
            }
            for x in 0..dw {
                let dst = c * dw * dh + out_row + x;
                if x < plan.x0 as usize || x >= plan.x0 as usize + plan.content_w as usize {
                    data[dst] = pad_norm;
                } else {
                    let value = resized[row_cursor];
                    row_cursor += 1;
                    data[dst] = ((value - mean) * inv_std).max(0.0);
                }
            }
        }
    }
    Ok(NormalizedTensor { data, plan })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::{DateTime, Utc};
    use common_types::EdgeStamp;
    use proptest::prelude::TestCaseError;
    use proptest::prelude::*;

    fn stamp() -> EdgeStamp {
        EdgeStamp::new(DateTime::<Utc>::UNIX_EPOCH, 0, 0, true)
    }

    fn frame(id: &'static str, w: u32, h: u32, fmt: PixelFormat, data: Vec<u8>) -> FrameView {
        FrameView {
            camera_id: CameraId::try_new(id).unwrap(),
            ts: stamp(),
            width: w,
            height: h,
            format: fmt,
            data,
        }
    }

    fn uniform_bgr(w: u32, h: u32, b: u8, g: u8, r: u8) -> FrameView {
        frame(
            "cam",
            w,
            h,
            PixelFormat::Bgr8,
            [b, g, r].repeat((w * h) as usize),
        )
    }

    #[test]
    fn identity_size_preserves_values() {
        let f = uniform_bgr(4, 4, 1, 2, 3);
        let out = preprocess(&f, ModelFamily::Rgb01, (4, 4)).unwrap();
        assert_eq!(out.plan.scale, 1.0);
        assert_eq!(out.plan.content_w, 4);
        assert_eq!(out.plan.content_h, 4);
        assert!((out.channel(0, 0, 0) - 3.0 / 255.0).abs() < 1e-4);
        assert!((out.channel(1, 3, 3) - 2.0 / 255.0).abs() < 1e-4);
        assert!((out.channel(2, 0, 3) - 1.0 / 255.0).abs() < 1e-4);
        assert_eq!(out.len(), 3 * 16);
    }

    #[test]
    fn letterbox_1920x1080_places_content_centered() {
        let f = uniform_bgr(1920, 1080, 0, 0, 0);
        let out = preprocess(&f, ModelFamily::Rgb01, (640, 640)).unwrap();
        let scale = 640.0_f32 / 1920.0;
        let content_h = ((1080.0_f32 * scale).round() as u32).max(1);
        assert_eq!(out.plan.scale, scale);
        assert_eq!(out.plan.content_w, 640);
        assert_eq!(out.plan.content_h, content_h);
        assert_eq!(out.plan.x0, 0);
        assert_eq!(out.plan.y0, (640 - content_h) / 2);
        // Corners are padding (gray mid-tone), content center is zero.
        assert!(out.channel(0, 0, 320) > 0.1);
        let mid_y = (out.plan.y0 + content_h / 2) as usize;
        assert!(out.channel(0, mid_y, 320).abs() < 1e-5);
        assert_eq!(out.len(), 3 * 640 * 640);
    }

    #[test]
    fn bgr_to_rgb_channel_order() {
        // A red (R=255) pixel at (2,1) encoded as BGR8 [b=0, g=0, r=255].
        let mut data = vec![0u8; 3 * 4 * 3];
        data[6 * 3 + 2] = 255;
        let f = frame("cam", 4, 3, PixelFormat::Bgr8, data);
        let out = preprocess(&f, ModelFamily::Rgb01, (4, 3)).unwrap();
        assert!((out.channel(0, 1, 2) - 1.0).abs() < 1e-4);
        assert!(out.channel(1, 1, 2).abs() < 1e-4);
        assert!(out.channel(2, 1, 2).abs() < 1e-4);
        assert!(out.channel(0, 0, 0).abs() < 1e-4);
    }

    #[test]
    fn imagenet_family_normalizes_reference_gray_to_zero() {
        // Reference ImageNet gray in BGR order: B=104, G=117, R=124.
        let f = uniform_bgr(2, 2, 104, 117, 124);
        let out = preprocess(&f, ModelFamily::ImageNet, (2, 2)).unwrap();
        assert!(out.channel(0, 0, 0).abs() < 1e-2);
        assert!(out.channel(1, 1, 0).abs() < 1e-2);
        assert!(out.channel(2, 1, 1).abs() < 1e-2);
    }

    #[test]
    fn nv12_uses_luma_plane_only() {
        let w = 4u32;
        let h = 4u32;
        let pixels = w * h;
        let mut data = vec![200u8; (pixels + pixels / 2) as usize];
        data[0] = 100;
        let f = frame("cam", w, h, PixelFormat::Nv12, data);
        let out = preprocess(&f, ModelFamily::Rgb01, (4, 4)).unwrap();
        assert!((out.channel(0, 0, 0) - 100.0 / 255.0).abs() < 1e-3);
        assert!((out.channel(2, 3, 3) - 200.0 / 255.0).abs() < 1e-3);
    }

    #[test]
    fn gray_replicates_to_all_planes() {
        let f = frame("cam", 2, 2, PixelFormat::Gray8, vec![10u8, 20, 30, 40]);
        let out = preprocess(&f, ModelFamily::Rgb01, (2, 2)).unwrap();
        assert!((out.channel(0, 0, 0) - 10.0 / 255.0).abs() < 1e-6);
        assert!((out.channel(1, 1, 1) - 40.0 / 255.0).abs() < 1e-6);
        assert!((out.channel(2, 0, 1) - 20.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn wrong_buffer_len_is_rejected() {
        let f = frame("cam", 4, 4, PixelFormat::Bgr8, vec![0u8; 3 * 4 * 4 + 1]);
        assert!(matches!(
            preprocess(&f, ModelFamily::Rgb01, (4, 4)),
            Err(InferenceError::InvalidFrameBuffer { .. })
        ));
    }

    #[test]
    fn oversized_frame_is_rejected_without_giant_allocation() {
        // Width far beyond the pixel budget with a tiny buffer: the pixel
        // gate must fire before any length comparison.
        let huge = u32::try_from(MAX_SOURCE_PIXELS / 4).unwrap() * 4 + 8;
        let f = frame("cam", huge, 32, PixelFormat::Bgr8, vec![0u8; 96]);
        assert!(matches!(
            preprocess(&f, ModelFamily::Rgb01, (640, 640)),
            Err(InferenceError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn zero_dims_rejected() {
        let f = frame("cam", 0, 0, PixelFormat::Bgr8, vec![]);
        assert!(matches!(
            preprocess(&f, ModelFamily::Rgb01, (4, 4)),
            Err(InferenceError::EmptyFrame { .. })
        ));
        let g = uniform_bgr(8, 8, 0, 0, 0);
        assert!(preprocess(&g, ModelFamily::Rgb01, (0, 640)).is_err());
    }

    #[test]
    fn unmap_roundtrips_known_box() {
        let plan = LetterboxPlan::compute((1920, 1080), (640, 640)).unwrap();
        let padded = [
            plan.x0 as f32,
            plan.y0 as f32,
            (plan.x0 + plan.content_w) as f32,
            (plan.y0 + plan.content_h) as f32,
        ];
        let back = plan.unmap_xyxy(padded);
        assert_eq!(back[0], 0.0);
        assert_eq!(back[1], 0.0);
        assert!((back[2] - 1920.0).abs() < 1.0);
        assert!((back[3] - 1080.0).abs() < 1.0);
    }

    proptest! {
        #[test]
        fn letterbox_geometry_covers_either_edge(
            sw in 4u32..=4000, sh in 4u32..=3000, tw in 64u32..=1024, th in 64u32..=1024
        ) {
            let plan = LetterboxPlan::compute((sw, sh), (tw, th))
                .ok_or_else(|| TestCaseError::fail("degenerate dims"))?;
            let content_w = ((sw as f32 * plan.scale).round() as u32).min(tw).max(1);
            let content_h = ((sh as f32 * plan.scale).round() as u32).min(th).max(1);
            prop_assert_eq!(plan.content_w, content_w);
            prop_assert_eq!(plan.content_h, content_h);
            prop_assert_eq!(plan.x0, (tw - content_w) / 2);
            prop_assert_eq!(plan.y0, (th - content_h) / 2);
            let fits_w = plan.content_w <= tw && plan.content_w > 0;
            let fits_h = plan.content_h <= th && plan.content_h > 0;
            prop_assert!(fits_w && fits_h);
            // Aspect-preservation edge: one axis must exactly fill the target.
            let filled_w = (content_w as f32 - f32::from(u16::try_from(sw).map_err(|_| TestCaseError::fail("sw>64k")).unwrap_or(u16::MAX)) * plan.scale).abs() < 1.0;
            let filled_h = (content_h as f32 - f32::from(u16::try_from(sh).map_err(|_| TestCaseError::fail("sh>64k")).unwrap_or(u16::MAX)) * plan.scale).abs() < 1.0;
            prop_assert!(filled_w || filled_h);
        }
    }
}
