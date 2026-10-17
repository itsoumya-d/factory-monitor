//! Source abstraction + fixture contracts (docs/04 §7).
//!
//! CI fakes swap in through this trait exactly as specified in
//! docs/14 §2 (sources deterministic, no `Utc::now()` inside unit paths).

use async_trait::async_trait;

use crate::error::CctvError;
use common_types::FrameView;

/// Nominal properties a source declares at open time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceProps {
    pub fps: f32,
    pub width: u32,
    pub height: u32,
}

/// One camera's decode source. `next_frame` returns:
/// * `Some(Ok(view))` — one decoded frame,
/// * `None` — orderly end-of-stream (fixture exhaustion / camera shutdown),
/// * `Some(Err)` — attempt failed; the pipeline applies watchdog backoff.
#[async_trait]
pub trait SourceAdapter: Send {
    async fn next_frame(&mut self) -> Option<Result<FrameView, CctvError>>;

    fn props(&self) -> SourceProps {
        SourceProps {
            fps: 30.0,
            width: 64,
            height: 36,
        }
    }
}
