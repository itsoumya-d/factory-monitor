//! A decoded camera frame handed downward to inference (docs/04 §1), plus the
//! basic pixel layout info needed for letterboxing.

use serde::{Deserialize, Serialize};

use crate::edge_stamp::EdgeStamp;
use crate::ids::CameraId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgr8,
    Bgrx8,
    Nv12,
    Gray8,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(&self) -> u8 {
        match self {
            Self::Bgr8 => 3,
            Self::Bgrx8 => 4,
            Self::Gray8 => 1,
            // 12 bits per pixel over two planes averaged down.
            Self::Nv12 => 12 / 8,
        }
    }
}

/// Ownership stays with the ingestion ring; `data` is a detached copy when
/// crossing thread boundaries (`FrameView` is cheap-clone in hot paths).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameView {
    pub camera_id: CameraId,
    pub ts: EdgeStamp,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl FrameView {
    /// Validate the buffer length against the declared geometry.
    pub fn validate(&self) -> bool {
        match self.width.checked_mul(self.height) {
            Some(pixels) => match usize::try_from(u64::from(pixels)) {
                Ok(p) => p * usize::from(self.format.bytes_per_pixel()) == self.data.len(),
                Err(_) => false,
            },
            None => false,
        }
    }
}
