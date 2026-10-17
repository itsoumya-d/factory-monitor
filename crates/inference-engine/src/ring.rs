//! Hard-negative ring buffer (docs/07 §6): the in-memory evidence ring the
//! nightly VLM/labeling pipeline (docs/09 §6) walks. Priorities enforce
//! divergence > occlusion > plain low-conf protection under memory budget.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};

use common_types::{CameraId, ZoneId};

/// RAM budget of the ring (docs/07 §6: 1 GB in-memory; NVMe spill needs a
/// filesystem sink and lands with the deployment crate).
pub const DEFAULT_BUDGET_BYTES: usize = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NegativePriority {
    /// Ordinary low-confidence frames.
    Plain,
    /// Frames the engine interpolated through an occlusion.
    Occlusion,
    /// Frames where modalities disagreed — the top label value.
    Divergence,
}

impl NegativePriority {
    pub fn categorize(divergence: f32, occluded: bool) -> Self {
        if divergence.is_finite() && divergence > 3.0 {
            Self::Divergence
        } else if occluded {
            Self::Occlusion
        } else {
            Self::Plain
        }
    }
}

/// Promotion states (docs/07 §6): Amber → Uploaded → AutoLabeled →
/// LabeledOK → TrainedEpoch{v}. The nightly pipeline advances these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionState {
    Amber,
    Uploaded,
    AutoLabeled,
    LabeledOK,
    TrainedEpoch { version: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HardNegativeRecord {
    pub ts: DateTime<Utc>,
    pub zone_id: Option<ZoneId>,
    pub camera_id: CameraId,
    /// Low-res frame bytes (the ring stores resized evidence, docs/07 §6).
    pub frame_bytes: Vec<u8>,
    /// CSI window bytes when available.
    pub csi_bytes: Option<Vec<u8>>,
    /// Modality divergence magnitude at capture time.
    pub divergence: f32,
    /// Tracks were occluded when captured.
    pub occluded: bool,
    pub promotion: PromotionState,
}

impl HardNegativeRecord {
    pub fn byte_len(&self) -> usize {
        self.frame_bytes.len() + self.csi_bytes.as_ref().map_or(0, Vec::len)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    priority: NegativePriority,
    record: HardNegativeRecord,
}

pub struct HardNegativeRing {
    budget_bytes: usize,
    live_bytes: usize,
    entries: VecDeque<Entry>,
    rejected: u64,
}

impl HardNegativeRing {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes: budget_bytes.max(1),
            live_bytes: 0,
            entries: VecDeque::new(),
            rejected: 0,
        }
    }

    pub fn with_default_budget() -> Self {
        Self::new(DEFAULT_BUDGET_BYTES)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    pub fn rejected_count(&self) -> u64 {
        self.rejected
    }

    pub fn iter(&self) -> impl Iterator<Item = &HardNegativeRecord> {
        self.entries.iter().map(|e| &e.record)
    }

    /// `try_send` semantics (docs/07 §6): a full ring drops the new record
    /// unless the oldest run of strictly-lower-priority entries frees enough
    /// room, which the eviction then removes (oldest-first, stopping at the
    /// first entry of equal-or-higher priority).
    pub fn push(&mut self, record: HardNegativeRecord, priority: NegativePriority) -> bool {
        let cost = record.byte_len();
        if cost > self.budget_bytes {
            self.rejected += 1;
            return false;
        }
        if self.live_bytes + cost <= self.budget_bytes {
            self.entries.push_back(Entry { priority, record });
            self.live_bytes += cost;
            return true;
        }
        let needed = self.live_bytes + cost - self.budget_bytes;
        let mut freed = 0usize;
        let mut evict_count = 0usize;
        for entry in &self.entries {
            if freed >= needed || entry.priority >= priority {
                break;
            }
            freed += entry.record.byte_len();
            evict_count += 1;
        }
        if freed < needed {
            self.rejected += 1;
            return false;
        }
        for _ in 0..evict_count {
            if let Some(entry) = self.entries.pop_front() {
                self.live_bytes -= entry.record.byte_len();
            }
        }
        self.entries.push_back(Entry { priority, record });
        self.live_bytes += cost;
        true
    }

    /// Oldest-first handoff to the nightly pipeline.
    pub fn drain(&mut self, max_records: usize) -> Vec<HardNegativeRecord> {
        let mut out = Vec::new();
        for _ in 0..max_records {
            match self.entries.pop_front() {
                Some(entry) => {
                    self.live_bytes -= entry.record.byte_len();
                    out.push(entry.record);
                }
                None => break,
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::Utc;
    use common_types::ids::{CameraId, ZoneId};

    fn record(id: u8, frame_len: usize) -> HardNegativeRecord {
        HardNegativeRecord {
            ts: Utc::now(),
            zone_id: ZoneId::try_new("z").ok(),
            camera_id: CameraId::try_new("cam").unwrap(),
            frame_bytes: vec![id; frame_len],
            csi_bytes: None,
            divergence: 0.0,
            occluded: false,
            promotion: PromotionState::Amber,
        }
    }

    fn camera(id: &str) -> CameraId {
        CameraId::try_new(id).unwrap()
    }

    #[test]
    fn accepts_within_budget_order_preserved() {
        let mut ring = HardNegativeRing::new(1_000);
        for _ in 0..5 {
            assert!(ring.push(record(1, 100), NegativePriority::Plain));
        }
        assert_eq!(ring.len(), 5);
        assert_eq!(ring.live_bytes(), 500);
        let drained = ring.drain(3);
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].frame_bytes[0], 1, "FIFO order");
        assert_eq!(ring.live_bytes(), 200);
    }

    #[test]
    fn rejects_equal_priority_when_full() {
        let mut ring = HardNegativeRing::new(300);
        assert!(ring.push(record(1, 150), NegativePriority::Plain));
        assert!(ring.push(record(2, 150), NegativePriority::Plain));
        assert!(!ring.push(record(3, 150), NegativePriority::Plain));
        assert_eq!(ring.rejected_count(), 1);
        assert_eq!(ring.len(), 2);
    }

    #[test]
    fn evicts_oldest_lower_priority_until_fit() {
        let mut ring = HardNegativeRing::new(400);
        for _ in 0..4 {
            ring.push(record(1, 100), NegativePriority::Plain);
        }
        // Plain flood (400 B). A Divergence record of 300 B needs 300 B
        // freed: the three oldest Plain entries go.
        assert!(ring.push(record(9, 300), NegativePriority::Divergence));
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.live_bytes(), 400);
        let drained = ring.drain(2);
        assert_eq!(drained[0].frame_bytes[0], 1, "last plain survives first");
        assert_eq!(drained[1].frame_bytes[0], 9, "then the divergence record");
        assert_eq!(ring.rejected_count(), 0);
    }

    #[test]
    fn stops_evicting_at_equal_priority() {
        let mut ring = HardNegativeRing::new(200);
        ring.push(record(1, 100), NegativePriority::Divergence);
        ring.push(record(2, 100), NegativePriority::Occlusion);
        // Occlusion cannot evict Divergence; the ring stays full.
        assert!(!ring.push(record(3, 150), NegativePriority::Occlusion));
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.rejected_count(), 1);
    }

    #[test]
    fn oversized_record_rejected_upfront() {
        let mut ring = HardNegativeRing::new(100);
        assert!(!ring.push(record(1, 500), NegativePriority::Divergence));
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn diagnostics_expose_counts() {
        let mut ring = HardNegativeRing::with_default_budget();
        assert!(ring.is_empty());
        ring.push(record(1, 10), NegativePriority::Plain);
        assert_eq!(ring.len(), 1);
        assert!(!ring.is_empty());
        assert!(ring.iter().next().is_some());
        assert_eq!(ring.drain(10).len(), 1);
        assert!(ring.drain(10).is_empty(), "drain exhausted ring");
    }

    #[test]
    fn categorize_ranks_by_divergence_then_occlusion() {
        assert_eq!(
            NegativePriority::categorize(3.5, false),
            NegativePriority::Divergence
        );
        assert_eq!(
            NegativePriority::categorize(0.0, true),
            NegativePriority::Occlusion
        );
        assert_eq!(
            NegativePriority::categorize(0.0, false),
            NegativePriority::Plain
        );
        assert_eq!(
            NegativePriority::categorize(f32::NAN, true),
            NegativePriority::Occlusion,
            "non-finite divergence is not rank inflation"
        );
    }

    #[test]
    fn csi_bytes_count_toward_cost() {
        let mut rec = record(1, 100);
        rec.csi_bytes = Some(vec![0; 100]);
        assert_eq!(rec.byte_len(), 200);
    }

    #[test]
    fn promotion_chain_states() {
        assert_eq!(PromotionState::Amber, PromotionState::Amber);
        assert_ne!(
            PromotionState::TrainedEpoch { version: 2 },
            PromotionState::TrainedEpoch { version: 3 }
        );
        let _ = camera("cam");
    }
}
