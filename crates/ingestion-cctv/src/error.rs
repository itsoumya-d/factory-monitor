use std::path::PathBuf;

#[derive(Debug, Clone, thiserror::Error)]
pub enum CctvError {
    #[error("frame geometry {width}x{height} is below the 64x36 luminance floor")]
    FrameTooSmall { width: u32, height: u32 },
    #[error("fixture '{path}': {reason}")]
    CorruptFixture { path: PathBuf, reason: String },
    #[error("camera '{0}' needs a positive luma gain")]
    InvalidLumaGain(String),
    #[error("camera '{0}' has an empty source configuration")]
    EmptySource(String),
    #[error("camera '{0}' is configured twice in the camera set")]
    DuplicateCamera(String),
}

impl From<image::ImageError> for CctvError {
    fn from(err: image::ImageError) -> Self {
        CctvError::CorruptFixture {
            path: PathBuf::new(),
            reason: err.to_string(),
        }
    }
}
