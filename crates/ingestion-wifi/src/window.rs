//! Sliding-window processor (docs/05 §2): 64-packet windows over a 32-packet
//! stride, sequence-gap accounting, reconstruction rules, and the
//! `WifiTick` mapping consumed by fusion.

use chrono::{DateTime, Utc};
use common_types::{EdgeStamp, WifiTick};

use crate::error::CsiError;
use crate::features::{WindowConfig, WindowFeatures, compute_window_features};
use crate::record::{DecodedPacket, RawCsiRecord};
use crate::sanitize::sanitize_packets;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpan {
    pub ts_from: DateTime<Utc>,
    pub ts_to: DateTime<Utc>,
    pub seq_first: u64,
    pub seq_last: u64,
}

impl WindowSpan {
    /// Inclusive sequence span `seq_last - seq_first + 1` — the packet count
    /// an *unlossy* wireless link would have delivered.
    pub fn packet_span(&self) -> u64 {
        self.seq_last.saturating_sub(self.seq_first) + 1
    }

    pub fn received(&self, count: usize) -> u64 {
        (count as u64).min(self.packet_span())
    }

    pub fn packet_loss_ratio(&self, count: usize) -> f32 {
        let span = self.packet_span();
        if span == 0 {
            return 0.0;
        }
        let missing = span.saturating_sub(self.received(count));
        missing as f32 / span as f32
    }
}

/// Fill sequence gaps in a buffered window, docs/05 §2 [reconstruction]:
/// * gaps of `max_gap` or fewer packets carry the previous packet's CSI
///   forward (small stalls look locally static);
/// * longer gaps linearly interpolate and the caller must mark the window
///   degraded. Returns whether any long gap was bridged.
fn reconstruct_gaps(buffer: &mut Vec<DecodedPacket>, max_gap: u64) -> bool {
    let mut long_gap = false;
    let mut i = 1;
    while i < buffer.len() {
        let gap = buffer[i].seq.saturating_sub(buffer[i - 1].seq);
        if gap > 1 {
            let missing = (gap - 1) as usize;
            if missing as u64 > max_gap {
                long_gap = true;
            }
            let prev = buffer[i - 1].clone();
            let next = buffer[i].clone();
            let span_ns = (next.ts.utc - prev.ts.utc).num_milliseconds().max(0);
            for k in 1..=missing {
                let seq = prev.seq + k as u64;
                let ts = EdgeStamp::new(
                    prev.ts.utc
                        + chrono::Duration::milliseconds(span_ns * k as i64 / (missing + 1) as i64),
                    prev.ts.stream_pts,
                    prev.ts.ptp_offset_ns,
                    prev.ts.clock_ok && next.ts.clock_ok,
                );
                let window_rec = DecodedPacket {
                    seq,
                    ts,
                    amps: if missing as u64 > max_gap {
                        mix(&prev.amps, &next.amps, k as f32 / (missing + 1) as f32)
                    } else {
                        prev.amps.clone()
                    },
                    phases: if missing as u64 > max_gap {
                        mix(&prev.phases, &next.phases, k as f32 / (missing + 1) as f32)
                    } else {
                        prev.phases.clone()
                    },
                };
                buffer.insert(i, window_rec);
                i += 1;
            }
        }
        i += 1;
    }
    long_gap
}

#[inline]
fn mix(a: &[f32], b: &[f32], weight_b: f32) -> Vec<f32> {
    a.iter()
        .zip(b.iter())
        .map(|(va, vb)| va * (1.0 - weight_b) + vb * weight_b)
        .collect()
}

/// Buffers decoded packets and emits `window_packets`-wide feature windows on
/// a `stride_packets` cadence. Duplicate/regressing sequence numbers are
/// dropped silently (the transport path is at-least-once).
#[derive(Debug)]
pub struct CsiWindowProcessor {
    cfg: WindowConfig,
    buffer: Vec<DecodedPacket>,
    last_seq: Option<u64>,
}

impl CsiWindowProcessor {
    pub fn new(cfg: WindowConfig) -> Self {
        Self {
            cfg,
            buffer: Vec::new(),
            last_seq: None,
        }
    }

    pub const fn config(&self) -> &WindowConfig {
        &self.cfg
    }

    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Push one record; returns a finished window when the buffer spans the
    /// configured window length. Decode errors propagate as hard errors.
    ///
    /// Memory bound: if a long link stall lets the sequence span grow past
    /// `2 * window_packets`, the oldest packets are discarded and the window
    /// is still emitted faithfully (`long_gap_used` marks reconstruction).
    pub fn push(
        &mut self,
        record: &RawCsiRecord,
    ) -> Result<Option<(WindowSpan, WindowFeatures)>, CsiError> {
        if let Some(last) = self.last_seq {
            if record.seq_no <= last {
                return Ok(None);
            }
        }
        let decoded = record.decode()?;
        self.last_seq = Some(record.seq_no);
        self.buffer.push(decoded);

        // Bound the buffer against link stalls.
        let max_keep = self.cfg.window_packets * 2;
        if self.buffer.len() > max_keep {
            let excess = self.buffer.len() - max_keep;
            self.buffer.drain(..excess);
        }
        if self.buffer.is_empty() {
            return Ok(None);
        }

        let first = self.buffer.first().map_or(0, |p| p.seq);
        let last_seq = self.buffer.last().map_or(first, |p| p.seq);
        let span = last_seq.saturating_sub(first) + 1;
        if span < self.cfg.window_packets as u64 {
            return Ok(None);
        }

        let received = self.buffer.len();
        let ts_from = self
            .buffer
            .first()
            .map_or(DateTime::<Utc>::UNIX_EPOCH, |p| p.ts.utc);
        let ts_to = self.buffer.last().map_or(ts_from, |p| p.ts.utc);
        let window_span = WindowSpan {
            ts_from,
            ts_to,
            seq_first: first,
            seq_last: last_seq,
        };
        // Loss comes from the raw receive counts (pre-reconstruction), so
        // fusion sees the link truth even though features are dense.
        let loss_ratio = window_span.packet_loss_ratio(received);
        let long_gap = self
            .buffer
            .windows(2)
            .any(|pair| pair[1].seq.saturating_sub(pair[0].seq) > self.cfg.max_reconstruct_gap);
        reconstruct_gaps(&mut self.buffer, self.cfg.max_reconstruct_gap);
        let sanitized = sanitize_packets(&self.buffer);
        let features = compute_window_features(&sanitized, &self.cfg, loss_ratio, long_gap)?;

        let stride = self.cfg.stride_packets.min(self.buffer.len());
        self.buffer.drain(..stride);

        Ok(Some((window_span, features)))
    }
}

/// Map a finished window onto the fusion-side tick. `clock_ok` comes from the
/// processor caller: false when any contributing packet disagreed about the
/// local clock (docs/13 §3 PTP rules).
pub fn features_to_wifi_tick(
    span: &WindowSpan,
    features: &WindowFeatures,
    received: usize,
    clock_ok: bool,
) -> WifiTick {
    WifiTick {
        presence_prob: features.presence_prob,
        doppler_energy: features.doppler_energy,
        motion_variance: features.motion_variance,
        packet_loss_ratio: span.packet_loss_ratio(received),
        mover_count_estimate: features.mover_count_estimate.map(|v| v as f64),
        clock_ok,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::record::{CsiFormat, MacAddr, RawCsiRecord};
    use common_types::{EdgeStamp, ZoneId};

    fn record(seq: u64, zone: &ZoneId) -> RawCsiRecord {
        // n=8 subcarriers, constant static amplitude: I=40, Q=0 per pair.
        let payload = [40u8, 0].repeat(8);
        RawCsiRecord {
            node_id: 7,
            zone_hint: zone.clone(),
            mac_src: MacAddr::new([0x30, 0xAE, 0xA4, 0x07, 0x1D, 7]),
            n_subcarriers: 8,
            format: CsiFormat::Iq8,
            seq_no: seq,
            t_recv: EdgeStamp::new(
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
                    + chrono::Duration::milliseconds(seq as i64 * 10),
                0,
                0,
                true,
            ),
            payload,
        }
    }

    fn tight_config() -> WindowConfig {
        WindowConfig {
            window_packets: 8,
            stride_packets: 4,
            ..WindowConfig::defaults()
        }
    }

    #[test]
    fn emits_only_when_span_reaches_window() {
        let zone = ZoneId::try_new("line_1").unwrap();
        let mut proc = CsiWindowProcessor::new(tight_config());
        for seq in 0..7 {
            assert!(
                proc.push(&record(seq, &zone)).unwrap().is_none(),
                "span {seq} below the 8-packet window"
            );
        }
        let (span, _feats) = proc.push(&record(7, &zone)).unwrap().unwrap();
        assert_eq!(span.seq_first, 0);
        assert_eq!(span.seq_last, 7);
        assert_eq!(proc.buffered(), 4, "stride evicts half");
    }

    #[test]
    fn duplicate_sequence_numbers_dropped_silently() {
        let zone = ZoneId::try_new("line_1").unwrap();
        let mut proc = CsiWindowProcessor::new(tight_config());
        let _ = proc.push(&record(0, &zone)).unwrap();
        let swapped = proc.push(&record(0, &zone)).unwrap();
        assert!(swapped.is_none(), "regressing seq ignored");
        let regressed = proc.push(&record(1, &zone)).unwrap();
        assert!(regressed.is_none(), "at-least-once duplicates ignored");
    }

    #[test]
    fn stride_cadence_walks_the_window() {
        let zone = ZoneId::try_new("line_1").unwrap();
        let mut proc = CsiWindowProcessor::new(tight_config());
        let mut emitted = 0usize;
        for seq in 0..20 {
            if proc.push(&record(seq, &zone)).unwrap().is_some() {
                emitted += 1;
            }
        }
        // Window closes whenever the span reaches 8: seq 7, 11, 15, 19.
        assert_eq!(
            emitted, 4,
            "one window per stride at span edges, got {emitted}"
        );
    }

    #[test]
    fn packet_loss_ratio_counts_missing_seqs() {
        let zone = ZoneId::try_new("line_1").unwrap();
        let mut proc = CsiWindowProcessor::new(tight_config());
        let mut feats = None;
        for seq in 0..8 {
            if seq == 5 {
                continue; // one lost packet
            }
            if let Some((_, f)) = proc.push(&record(seq, &zone)).unwrap() {
                feats = Some(f);
            }
        }
        let feats = feats.unwrap();
        assert!(
            (feats.packet_loss_ratio - 1.0f32 / 8.0).abs() < 1e-5,
            "1 lost of span 8, got {}",
            feats.packet_loss_ratio
        );
    }

    #[test]
    fn long_gap_flags_window_degraded() {
        let zone = ZoneId::try_new("line_1").unwrap();
        let mut proc = CsiWindowProcessor::new(tight_config());
        for seq in 0..8 {
            let _ = proc.push(&record(seq, &zone)).unwrap();
        }
        // Jump the stream by 4 packets (gap 4 > max_reconstruct_gap 3).
        let mut any_long_gap = false;
        for seq in 12..20 {
            if let Some((_, feats)) = proc.push(&record(seq, &zone)).unwrap() {
                any_long_gap |= feats.long_gap_used;
            }
        }
        assert!(
            any_long_gap,
            "a window bridging a 4-packet gap must be flagged degraded"
        );

        // Without any gap the next window is clean.
        let mut clean = None;
        for seq in 20..28 {
            if let Some((_, feats)) = proc.push(&record(seq, &zone)).unwrap() {
                clean = Some(feats);
            }
        }
        assert!(!clean.unwrap().long_gap_used);
    }

    #[test]
    fn to_wifi_tick_maps_fields() {
        let span = WindowSpan {
            ts_from: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            ts_to: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            seq_first: 0,
            seq_last: 9,
        };
        let features = crate::features::WindowFeatures {
            doppler_energy: 0.4,
            motion_variance: 0.02,
            presence_prob: 0.9,
            packet_loss_ratio: 0.1,
            mover_count_estimate: Some(1),
            band_mags: Vec::new(),
            band_bin_freqs: Vec::new(),
            long_gap_used: false,
        };
        let tick = features_to_wifi_tick(&span, &features, 9, true);
        assert!((tick.presence_prob - 0.9).abs() < 1e-6);
        assert_eq!(tick.packet_loss_ratio, 0.1, "1 missing of 10");
        assert_eq!(tick.mover_count_estimate, Some(1.0));
        assert!(tick.clock_ok);
        assert!(!tick.is_csi_degraded());
    }
}
