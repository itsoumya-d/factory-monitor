//! Shado CCTV ingestion (docs/04).
//!
//! Fixture-first build: the `SourceAdapter` trait lets CI run the whole
//! pipeline on file-backed and synthetic sources with zero GPU/native deps;
//! the GStreamer/NVDEC production source lands behind a later `gst` feature
//! when the hardware phase (docs/15 §2 Phase 8) wires the same trait in.

pub mod error;
pub mod filesrc;
pub mod ingestor;
pub mod luma;
pub mod ring;
pub mod source;
pub mod ssim;
pub mod synthetic;
pub mod trigger;
pub mod watchdog;

pub use error::CctvError;
pub use filesrc::{FileFrameSource, fixture_start};
pub use ingestor::{
    CameraPipeline, CameraSet, IngestEvent, SYSTEM_QUORUM_CAMERA, SpawnedPipelines, SpawnerLimits,
    StreamDegradationReason, TriggerParams, spawn_all,
};
pub use luma::{downsample_to_64x36, luma_plane, lux_proxy};
pub use ring::{FrameRing, RingPushOutcome};
pub use source::{SourceAdapter, SourceProps};
pub use ssim::{BLOCK_GRID, BlockRef, CHANGED_DELTA, block_ssim, flagged_blocks};
pub use synthetic::{SyntheticFrameSource, SyntheticScene};
pub use trigger::{CrossDirection, LuminanceQuorum, LuminanceTrigger};
pub use watchdog::{MAX_BACKOFF, backoff_delay};
