//! MoQT (Media over QUIC Transport) seam: subscription, instant rewind
//! via the cached FETCH window, and one-shot publish intent.

use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoqtError {
    #[error("window index {0} precedes the cached window")]
    WindowEvicted(u64),
    #[error("rewind cache bounded to {0} frames")]
    CacheFull(usize),
    #[error("track must have a name")]
    NoTrackName,
}

/// Transport seam; the real MoQT client is feature-gated (not in CI).
pub trait MoqtTransport: Send + Sync {
    fn subscribe(&self, track: &str) -> Result<(), MoqtError>;
    fn fetch(&self, track: &str, from_index: u64) -> Result<Vec<u8>, MoqtError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct CachedFrame {
    pub index: u64,
    pub payload: Vec<u8>,
}

/// A track-scoped rewind cache: lowest/evicted window tracking, all
/// bounded; state kept in RAM, no disk persistence.
#[derive(Debug, Clone, PartialEq)]
pub struct MoqtBus {
    pub track: String,
    cache: VecDeque<CachedFrame>,
    capacity: usize,
}

impl MoqtBus {
    pub fn try_new(track: &str, capacity: usize) -> Result<Self, MoqtError> {
        if track.is_empty() {
            return Err(MoqtError::NoTrackName);
        }
        if capacity == 0 {
            return Err(MoqtError::CacheFull(0));
        }
        Ok(Self {
            track: track.to_string(),
            cache: VecDeque::new(),
            capacity,
        })
    }

    pub fn push(&mut self, frame: CachedFrame) -> Result<(), MoqtError> {
        if frame.payload.is_empty() {
            return Err(MoqtError::NoTrackName);
        }
        if self.cache.len() == self.capacity {
            let _ = self.cache.pop_front();
        }
        self.cache.push_back(frame);
        Ok(())
    }

    /// Instant rewind: local cache hit; indexes inside the window are
    /// served without touching the transport.
    pub fn rewind_cached(&self, from_index: u64) -> Result<Vec<&CachedFrame>, MoqtError> {
        if let Some(oldest) = self.cache.front().map(|f| f.index)
            && from_index < oldest
        {
            return Err(MoqtError::WindowEvicted(from_index));
        }
        Ok(self
            .cache
            .iter()
            .filter(|f| f.index >= from_index)
            .collect())
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bus() -> MoqtBus {
        MoqtBus::try_new("cam/front", 4).expect("valid bus")
    }

    #[test]
    fn rewind_window_bounded() {
        let mut b = bus();
        for i in 0..6u64 {
            b.push(CachedFrame {
                index: i,
                payload: vec![i as u8],
            })
            .expect("fits");
        }
        assert_eq!(b.cache_len(), 4, "bounded after push beyond capacity");
        assert_eq!(b.rewind_cached(2).map(|v| v.len()), Ok(4));
        assert_eq!(b.rewind_cached(0), Err(MoqtError::WindowEvicted(0))); // was evicted
    }

    #[test]
    fn empty_track_name_is_rejected() {
        assert_eq!(MoqtBus::try_new("", 4), Err(MoqtError::NoTrackName));
    }
}
