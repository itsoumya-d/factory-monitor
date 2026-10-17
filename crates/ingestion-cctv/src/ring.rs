//! Frame ring buffer with drop-oldest semantics (docs/04 §3).
//!
//! Bounded, never blocks the producer (non-blocking `try_lock` busy path),
//! and reports per-push outcomes so the ingestor can account `dropped_frames`
//! metrics. The warranty shape is testable here (deterministic); the
//! zero-copy lock-free rework of the hot decode path lands with the hardware
//! phase (docs/04 §7).

use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use common_types::{CameraId, FrameRef, FrameView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingPushOutcome {
    Admitted,
    /// Oldest slot evicted to make room; carries its lazy-resolvable ref.
    AdmittedDroppedOldest {
        evicted: FrameRef,
    },
    /// Ring busy beyond the lock budget / poisoned mutex; newest frame
    /// dropped and accounted in `dropped_newest`.
    DroppedNewest,
}

#[derive(Debug)]
struct RingSlot {
    ring_seq: u64,
    view: FrameView,
}

#[derive(Debug)]
struct RingInner {
    slots: VecDeque<RingSlot>,
    capacity: usize,
    next_seq: u64,
    evicted_oldest: u64,
}

/// Per-camera FIFO of decoded frames. Consumers share it via `Arc`; the ring
/// itself is not cloned.
#[derive(Debug)]
pub struct FrameRing {
    camera: CameraId,
    /// Deliberately outside the mutex: `dropped_newest` records pushes that
    /// could not even acquire the lock.
    dropped_newest: AtomicU64,
    inner: Arc<Mutex<RingInner>>,
}

impl FrameRing {
    pub fn new(camera: CameraId, capacity: usize) -> Self {
        Self {
            camera,
            dropped_newest: AtomicU64::new(0),
            inner: Arc::new(Mutex::new(RingInner {
                slots: VecDeque::with_capacity(capacity.max(1)),
                capacity: capacity.max(1),
                next_seq: 0,
                evicted_oldest: 0,
            })),
        }
    }

    pub const fn camera(&self) -> &CameraId {
        &self.camera
    }

    /// Admit a frame; drop-oldest when at capacity. The busy path (mutex
    /// contended by a slow reader) drops the newest frame rather than blocking
    /// the ingest loop, mirroring the hardware-phase lock budget (docs/04 §3).
    pub fn push(&self, view: FrameView) -> RingPushOutcome {
        let mut guard = match self.inner.try_lock() {
            Ok(g) => g,
            Err(_) => {
                self.dropped_newest.fetch_add(1, Ordering::Relaxed);
                return RingPushOutcome::DroppedNewest;
            }
        };
        let ring_seq = guard.next_seq;
        if guard.slots.len() >= guard.capacity {
            match guard.slots.pop_front() {
                Some(slot) => {
                    guard.evicted_oldest += 1;
                    let evicted = FrameRef::new(self.camera.clone(), slot.ring_seq);
                    guard.slots.push_back(RingSlot { ring_seq, view });
                    guard.next_seq += 1;
                    return RingPushOutcome::AdmittedDroppedOldest { evicted };
                }
                None => return RingPushOutcome::Admitted,
            }
        }
        guard.slots.push_back(RingSlot { ring_seq, view });
        guard.next_seq += 1;
        RingPushOutcome::Admitted
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.slots.len()).unwrap_or(0)
    }

    pub fn evicted_oldest(&self) -> u64 {
        self.inner.lock().map(|g| g.evicted_oldest).unwrap_or(0)
    }

    pub fn dropped_newest(&self) -> u64 {
        self.dropped_newest.load(Ordering::Relaxed)
    }

    pub fn pop_oldest(&self) -> Option<FrameView> {
        self.inner
            .lock()
            .ok()?
            .slots
            .pop_front()
            .map(|slot| slot.view)
    }

    pub fn pop_newest(&self) -> Option<FrameView> {
        self.inner
            .lock()
            .ok()?
            .slots
            .pop_back()
            .map(|slot| slot.view)
    }

    pub fn drain_oldest_first(&self, max: usize) -> Vec<FrameView> {
        let Ok(mut guard) = self.inner.lock() else {
            return Vec::new();
        };
        let take = max.min(guard.slots.len());
        guard.slots.drain(..take).map(|slot| slot.view).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use common_types::{EdgeStamp, PixelFormat};

    fn frame(tag: u64) -> FrameView {
        FrameView {
            camera_id: CameraId::try_new("cam").unwrap(),
            ts: EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, tag, 0, true),
            width: 64,
            height: 36,
            format: PixelFormat::Gray8,
            data: vec![tag as u8; (64 * 36) as usize],
        }
    }

    #[test]
    fn drop_oldest_at_capacity_and_fifo_order() {
        let ring = FrameRing::new(CameraId::try_new("cam").unwrap(), 3);
        assert_eq!(ring.push(frame(0)), RingPushOutcome::Admitted);
        assert_eq!(ring.push(frame(1)), RingPushOutcome::Admitted);
        let third = ring.push(frame(2));
        assert_eq!(
            third,
            RingPushOutcome::Admitted,
            "capacity 3 holds three frames"
        );
        let fourth = ring.push(frame(3));
        assert!(
            matches!(fourth, RingPushOutcome::AdmittedDroppedOldest { evicted } if evicted.ring_seq == 0),
            "capacity 3 pushes a 4th -> oldest (seq 0) evicted"
        );
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.evicted_oldest(), 1);
        assert_eq!(ring.dropped_newest(), 0);

        let popped = [
            ring.pop_oldest().unwrap(),
            ring.pop_oldest().unwrap(),
            ring.pop_oldest().unwrap(),
        ];
        let order: Vec<u64> = popped.iter().map(|f| f.ts.stream_pts).collect();
        assert_eq!(order, vec![1, 2, 3], "oldest-first FIFO order preserved");
        assert!(ring.pop_oldest().is_none());
    }

    #[test]
    fn busy_lock_drops_newest_and_accounts_it() {
        let ring = Arc::new(FrameRing::new(CameraId::try_new("cam").unwrap(), 2));
        assert_eq!(ring.push(frame(0)), RingPushOutcome::Admitted);
        {
            // Hold the lock on another thread while the ingest loop pushes.
            let inner = Arc::clone(&ring);
            let holder = std::thread::spawn(move || {
                let _guard = inner.inner.lock().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(200));
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert_eq!(ring.push(frame(1)), RingPushOutcome::DroppedNewest);
            holder.join().unwrap();
        }
        assert_eq!(ring.dropped_newest(), 1);
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn drain_respects_max_argument() {
        let ring = FrameRing::new(CameraId::try_new("cam").unwrap(), 5);
        for i in 0..4 {
            let _ = ring.push(frame(i));
        }
        assert_eq!(ring.drain_oldest_first(2).len(), 2);
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.drain_oldest_first(99).len(), 2, "over-drain clamps");
        assert!(ring.drain_oldest_first(99).is_empty());
    }
}
