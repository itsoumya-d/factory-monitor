//! CSI source abstraction + synthetic generator (docs/05 §3, docs/14 §2).
//!
//! The `CsiSource` trait lives here (not in common-types): it needs tokio
//! channels, and common-types stays dependency-light. Every fixture source
//! replays deterministically; production UDP/serial transports implement the
//! same trait in the hardware phase.

use std::time::Duration;

use async_trait::async_trait;
use common_types::{EdgeStamp, ZoneId};
use tokio::select;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;

use crate::error::CsiError;
use crate::record::{CsiFormat, MacAddr, RawCsiRecord};

/// Abortion handle: drop-aware, single-shot.
#[derive(Debug)]
pub struct CsiHandle {
    abort: oneshot::Sender<()>,
}

impl CsiHandle {
    pub fn abort(self) {
        let _ = self.abort.send(());
    }
}

/// One CSI ingest source. Starts a background producer feeding `sink`; the
/// returned handle aborts it. Sources stop by themselves when `sink` closes.
#[async_trait]
pub trait CsiSource: Send {
    async fn start(self: Box<Self>, sink: Sender<RawCsiRecord>) -> Result<CsiHandle, CsiError>;
}

/// Replays pre-recorded [`RawCsiRecord`]s, optionally paced (e.g. at the
/// capture's 100 Hz rate). Order is preserved; pacing sleeps between sends.
pub struct ReplayCsiSource {
    records: Vec<RawCsiRecord>,
    pacing: Option<Duration>,
}

impl ReplayCsiSource {
    pub fn new(records: Vec<RawCsiRecord>, pacing: Option<Duration>) -> Self {
        Self { records, pacing }
    }
}

#[async_trait]
impl CsiSource for ReplayCsiSource {
    async fn start(mut self: Box<Self>, sink: Sender<RawCsiRecord>) -> Result<CsiHandle, CsiError> {
        let (abort_tx, mut abort_rx) = oneshot::channel();
        tokio::spawn(async move {
            for record in self.records.drain(..) {
                if let Some(pace) = self.pacing {
                    tokio::time::sleep(pace).await;
                } else {
                    tokio::task::yield_now().await;
                }
                select! {
                    _ = &mut abort_rx => return,
                    res = sink.send(record) => {
                        if res.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(CsiHandle { abort: abort_tx })
    }
}

/// One simulated mover: sinusoidal amplitude disturbance per subcarrier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimulatedMover {
    pub f_hz: f64,
    pub depth: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticCsiConfig {
    pub node_id: u8,
    pub zone_hint: ZoneId,
    pub n_subcarriers: u16,
    pub packet_rate_hz: f64,
    pub duration_s: f64,
    pub t0: EdgeStamp,
    pub movers: Vec<SimulatedMover>,
}

impl SyntheticCsiConfig {
    pub const fn defaults(zone_hint: ZoneId) -> Self {
        Self {
            node_id: 7,
            zone_hint,
            n_subcarriers: 64,
            packet_rate_hz: 100.0,
            duration_s: 10.0,
            t0: EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, 0, 0, true),
            movers: Vec::new(),
        }
    }
}

/// I/Q quantization note: the fixture generator writes the pcap float format
/// (`IqFloatPcap`) so amplitudes survive decode exactly; i8 quantization
/// noise (~1 LSB / CAScale) would swamp calibration-scale idle depths. The
/// i8 path is exercised by `record.rs` tests directly.
pub const CSI_IQ_SCALE: f32 = 40.0;

/// Deterministic workbook: for each packet index the base static amplitude 1.0
/// is disturbed by every mover's depth · cos(2π f t + subcarrier phase), then
/// encoded as an IqFloatPcap record that decodes to exact amplitudes.
pub fn generate_synthetic_records(cfg: &SyntheticCsiConfig) -> Vec<RawCsiRecord> {
    let count = (cfg.packet_rate_hz * cfg.duration_s).ceil() as usize;
    let mut records = Vec::with_capacity(count);
    let mac = MacAddr::new([0x30u8, 0xAE, 0xA4, 0x07, 0x1D, cfg.node_id]);
    let n = cfg.n_subcarriers as usize;
    for i in 0..count {
        let t = i as f64 / cfg.packet_rate_hz;
        let n_f = n as f64;
        let amps: Vec<f32> = (0..n)
            .map(|k| {
                let mut amp = 1.0f32;
                for mover in &cfg.movers {
                    let phase = std::f64::consts::TAU * mover.f_hz * t
                        + std::f64::consts::TAU * (k as f64) / n_f;
                    amp += (mover.depth as f32) * (phase as f32).cos();
                }
                amp.max(0.05)
            })
            .collect();
        let mut payload = Vec::with_capacity(8 * n);
        for (k, amp) in amps.iter().enumerate() {
            let theta = 0.37_f32 + 0.003 * k as f32;
            let amp = *amp * CSI_IQ_SCALE;
            payload.extend((amp * theta.cos()).to_le_bytes());
            payload.extend((amp * theta.sin()).to_le_bytes());
        }
        let utc = cfg.t0.utc + chrono::Duration::milliseconds((t * 1000.0).round() as i64);
        records.push(RawCsiRecord {
            node_id: cfg.node_id,
            zone_hint: cfg.zone_hint.clone(),
            mac_src: mac,
            n_subcarriers: cfg.n_subcarriers,
            format: CsiFormat::IqFloatPcap,
            seq_no: i as u64,
            t_recv: EdgeStamp::new(utc, i as u64, cfg.t0.ptp_offset_ns, cfg.t0.clock_ok),
            payload,
        });
    }
    records
}

/// Synthetic fixture source: pre-generates a deterministic stream and replays
/// it through [`ReplayCsiSource`].
pub struct SyntheticCsiSource {
    cfg: SyntheticCsiConfig,
    pacing: Option<Duration>,
}

impl SyntheticCsiSource {
    pub fn new(cfg: SyntheticCsiConfig, pacing: Option<Duration>) -> Self {
        Self { cfg, pacing }
    }
}

#[async_trait]
impl CsiSource for SyntheticCsiSource {
    async fn start(self: Box<Self>, sink: Sender<RawCsiRecord>) -> Result<CsiHandle, CsiError> {
        let repl = ReplayCsiSource::new(generate_synthetic_records(&self.cfg), self.pacing);
        Box::new(repl).start(sink).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::features::{WindowConfig, compute_window_features};
    use crate::sanitize::sanitize_packets;
    use crate::window::CsiWindowProcessor;
    use common_types::ZoneId;

    fn cfg_for(movers: Vec<SimulatedMover>) -> SyntheticCsiConfig {
        SyntheticCsiConfig {
            duration_s: 0.7, // 70 packets at 100 Hz
            movers,
            ..SyntheticCsiConfig::defaults(ZoneId::try_new("line_1").unwrap())
        }
    }

    #[tokio::test]
    async fn replay_source_feeds_records_then_ends() {
        let records = generate_synthetic_records(&cfg_for(vec![SimulatedMover {
            f_hz: 8.0,
            depth: 0.35,
        }]));
        assert_eq!(records.len(), 70);
        let (tx, mut rx) = tokio::sync::mpsc::channel(128);
        let source = Box::new(ReplayCsiSource::new(records, None));
        let handle = source.start(tx).await.unwrap();
        let mut received = 0usize;
        while let Some(_record) = rx.recv().await {
            received += 1;
            assert!(received <= 70, "source exceeds its recorded count");
            if received == 70 {
                break;
            }
        }
        assert_eq!(received, 70);
        handle.abort();
    }

    #[tokio::test]
    async fn abort_stops_the_stream_early() {
        let cfg = cfg_for(Vec::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let source = Box::new(SyntheticCsiSource::new(cfg, None));
        let handle = source.start(tx).await.unwrap();
        let mut few = 0usize;
        while few < 20 {
            if rx.recv().await.is_none() {
                break;
            }
            few += 1;
        }
        handle.abort();
        // Drain what is buffered; the source must stop emitting soon after.
        while rx.try_recv().is_ok() {}
    }

    /// Golden fixture (docs/05 §8): one synthetic walker present > 0.8
    /// presence, a static zone < 0.2 — through the whole ingest pipeline.
    #[tokio::test]
    async fn synthetic_walk_and_idle_presence_golden() {
        let zone = ZoneId::try_new("line_1").unwrap();
        struct Case {
            movers: Vec<SimulatedMover>,
            min: f32,
            max: f32,
            movers_expected: Option<u64>,
        }
        let cases = vec![
            Case {
                movers: vec![SimulatedMover {
                    f_hz: 8.0,
                    depth: 0.35,
                }],
                min: 0.8,
                max: 0.9999,
                movers_expected: Some(1),
            },
            Case {
                movers: vec![SimulatedMover {
                    f_hz: 0.2,
                    depth: 0.0001,
                }],
                min: 0.0,
                max: 0.2,
                movers_expected: None,
            },
        ];
        for case in &cases {
            let cfg = SyntheticCsiConfig {
                duration_s: 0.7,
                movers: case.movers.clone(),
                ..SyntheticCsiConfig::defaults(zone.clone())
            };
            let records = generate_synthetic_records(&cfg);
            let mut proc = CsiWindowProcessor::new(WindowConfig::defaults());
            let mut last = None;
            for record in &records {
                if let Some(out) = proc.push(record).unwrap() {
                    last = Some(out);
                }
            }
            let (span, feats) = last.unwrap_or_else(|| panic!("no window emitted"));
            assert!(
                (feats.packet_loss_ratio).abs() < 1e-6,
                "clean synthetic stream must show no loss"
            );
            assert!(
                feats.presence_prob > case.min && feats.presence_prob < case.max,
                "presence {} outside [{}, {}]",
                feats.presence_prob,
                case.min,
                case.max
            );
            assert_eq!(feats.mover_count_estimate, case.movers_expected);
            assert!(feats.motion_variance.is_finite());
            assert_eq!(span.seq_first, 0);
        }
    }

    /// The generator must be byte-deterministic across runs (docs/14).
    #[test]
    fn generator_is_deterministic() {
        let cfg = cfg_for(vec![SimulatedMover {
            f_hz: 8.0,
            depth: 0.35,
        }]);
        let a = generate_synthetic_records(&cfg);
        let b = generate_synthetic_records(&cfg);
        for (ra, rb) in a.iter().zip(b.iter()) {
            assert_eq!(ra.payload, rb.payload);
            assert_eq!(ra.seq_no, rb.seq_no);
        }
    }

    #[test]
    fn features_survive_sanitize_on_generated_stream() {
        let cfg = cfg_for(vec![SimulatedMover {
            f_hz: 8.0,
            depth: 0.35,
        }]);
        let records = generate_synthetic_records(&cfg);
        let packets: Vec<crate::record::DecodedPacket> = records
            .iter()
            .take(64)
            .map(|r| r.decode().unwrap())
            .collect();
        let window = sanitize_packets(&packets);
        let feats = compute_window_features(&window, &WindowConfig::defaults(), 0.0, false)
            .unwrap_or_else(|e| panic!("features on real decoded stream: {e}"));
        assert!(
            feats.presence_prob > 0.8,
            "decoded-s tream presence {} must stay walk-high",
            feats.presence_prob
        );
    }
}
