# 05 — Wi-Fi Sensing Crate (`ingestion-wifi`)

## 1. Why CSI

Cameras fail at: blind spots (pallet occlusion), steam/dust, glare, and low light. Wi-Fi CSI sees **through** occlusion: multipath reflections from 56–192 OFDM subcarriers encode motion — bodies and moving packages change amplitude/phase in characteristic patterns. We use CSI for:

- **zone presence** (is the zone occupied even if camera can't see it?),
- **motion cadence** (is the belt/work rhythm continuing during occlusion?), and
- **density estimate** (how many movers in the zone → correlated with package flow).

## 2. Hardware Source Abstraction

```rust
#[async_trait]
pub trait CsiSource: Send + Sync {
    /// Stream of raw CSI records. Implementations: UdpCsi (ESP32-S3 esp-csi FW),
    /// SerialCsi (esp-csi over USB-CDC), PcapCsi (AX210/5300 via pcap + vendor extractor).
    async fn start(&mut self, sink: mpsc::Sender<RawCsiRecord>) -> Result<CsiHandle, CsiError>;
}

pub struct RawCsiRecord {
    pub node_id: u8,            // 1..=4 ESP32 nodes or NIC index
    pub zone_hint: ZoneId,
    pub mac_src: MacAddr,
    pub n_subcarriers: u16,     // 56 (HT20) / 112 (HT40+) / 242 (HE20 AX210)
    pub format: CsiFormat,      // Iq8 | Iq10IeeeIwl | IqFloatPcap
    pub seq_no: u64,
    pub t_recv: DateTime<Utc>,  // edge receive stamp (PTP corrected)
    pub payload: Vec<u8>,       // raw iq stream bytes
}
```

**Node placement (per docs/13):** 2–4 nodes around a conveyor line (under-belt, above-belt, side-gantry); TX = the Wi-Fi AP, RX = nodes. Cost target <$30/node for ESP32-S3.

## 3. Preprocessing Pipeline (per zone, per sliding window)

```
RawCsi [] ──▶ [A] handle_size/order per format
        ──▶ [B] amplitude = sqrt(I²+Q²); phase = atan2(Q,I)
        ──▶ [C] phase sanitize (SUB-CARRIER-wise, 2 ops):
                c1) remove outlier packets: |Δamp| among (t-5..t+5) > 3.5σ_med → drop
                c2) unwrap phase over k (multi-subcarrier linear demodulator):
                    φ'(k) = φ(k) - φ(56) - (k/56)·(mean_phi_wedges)
        ──▶ [D] conjugate decorrelation over time t → amp_t, phase_t matrices [S×T]
        ──▶ [E] rustfft: FFT over time axis (window=64 packets) → mags [S×F]
        ──▶ [F] smooth: moving median 5 packets for denoise
        ──▶ [G] normalize: (x-μ)/σ per-subcarrier, debugging: write raw matrix to tap
        ──▶ [H] downsample: [64 subcarriers × 10 steps × 32 bytes] == model input tensor
```

## 4. Feature Extractors (outputs, all in common-types)

| Feature | Formula / method | Fused meaning |
|---|---|---|
| `doppler_energy` | Σ over F in [0..10 Hz] of \|FFT_mag\| | how much motion in window |
| `motion_variance` | Var(amp across T) per band | still vs. moving |
| `presence_prob` | softmax head of CSI 1D-CNN+Transformer ONNX (via ort) | person/mover present, handles occlusion |
| `mover_count_estimate` | spectral peak-count heuristic + model class-regression | 1–5 movers |
| `csi_variance` (feeds `FactoryEnvState`) | explained std of residual after smoothing | environment noise floor |
| `multipath_noise_level` | std of static windows over last 10 min | sickens the RL state (docs/08) |

## 5. Aligning CSI with Vision

- Both modalities use `EdgeStamp::utc` from the same PTP-corrected clock.
- `CrossModalAligner` (docs/07) matches windows: CSI windows vs. video frame at 100 ms stride.
- If PTP skew >25 ms reported, CSI is accepted but **degraded-marked** → fusion adds uncertainty.

## 6. Packet Loss Compensation

- `seq_no` gaps flagged. Reconstruct by carrying previous `amp_t` value forward if gap ≤ 3; on gap > 3, interpolate with linear ramp and set `consecutive_loss` flag.
- Tracked metric: `csi_packet_loss_ratio` over 60 s. Healthy <5% (ethernet-serial, UDP LAN).

## 7. Calibration Baseline (morning)

- During `Standby`, while lights are off (and thus `vision` is dead) we **establish the empty-floor CSI baseline**: per node/zone, capture 10-min matrix → record the per-subcarrier mean/σI basis. That becomes the zero-noise reference the day's divergence uses; it resets at each successful morning calibration.

## 8. Testing Hooks

- `CsiSource` has a `ReplayCsi` implementation reading `.npy`/binary fixtures captured on real floors — with a deterministic synthetic generator for CI (walking person at 0.5/1.0 m/s, forklift passes, 3-node mesh tests).
- Validated against ground truth in docs/14 §5 "cross-modal event burn-down test day".
