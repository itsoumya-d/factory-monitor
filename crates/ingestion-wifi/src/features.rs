//! Doppler / motion features (docs/05 §2 [G]–[H]).
//!
//! Amplitude scale handling: energy is AC-coupled (per-subcarrier mean
//! removal) but deliberately *not* sigma-normalized — presence must retain
//! absolute magnitude, otherwise a millimeter idle wobble normalizes to the
//! same spectrum as a forklift. Real-model normalization stays in Phase 3
//! (detector preprocessing), not in the presence heuristic.

use crate::error::CsiError;
use crate::sanitize::SanitizedWindow;

pub const MIN_WINDOW_PACKETS: usize = 8;

/// Sliding-window feature configuration (docs/05 §2 [G]).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowConfig {
    pub window_packets: usize,
    pub stride_packets: usize,
    pub packet_rate_hz: f64,
    pub doppler_min_hz: f64,
    pub doppler_max_hz: f64,
    pub presence_ref: f32,
    pub max_reconstruct_gap: u64,
}

impl WindowConfig {
    pub const fn defaults() -> Self {
        Self {
            window_packets: 64,
            stride_packets: 32,
            packet_rate_hz: 100.0,
            doppler_min_hz: 0.5,
            doppler_max_hz: 20.0,
            presence_ref: 0.01,
            max_reconstruct_gap: 3,
        }
    }

    pub fn validate(&self) -> Result<(), CsiError> {
        if self.window_packets < MIN_WINDOW_PACKETS {
            return Err(CsiError::InvalidConfig(format!(
                "window_packets {} below floor {MIN_WINDOW_PACKETS}",
                self.window_packets
            )));
        }
        if self.stride_packets == 0 || self.stride_packets > self.window_packets {
            return Err(CsiError::InvalidConfig(format!(
                "stride_packets {} must be in 1..={}",
                self.stride_packets, self.window_packets
            )));
        }
        if !(self.packet_rate_hz.is_finite() && self.packet_rate_hz > 0.0) {
            return Err(CsiError::InvalidConfig(format!(
                "packet_rate_hz {} must be positive",
                self.packet_rate_hz
            )));
        }
        if self.doppler_min_hz >= self.doppler_max_hz {
            return Err(CsiError::InvalidConfig(
                "doppler_min_hz must be < doppler_max_hz".to_string(),
            ));
        }
        if !(self.presence_ref.is_finite() && self.presence_ref > 0.0) {
            return Err(CsiError::InvalidConfig(
                "presence_ref must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowFeatures {
    pub doppler_energy: f32,
    pub motion_variance: f32,
    pub presence_prob: f32,
    /// Sequence-gap loss inside the emitted window (0.0..=1.0); owned here so
    /// downstream consumers see one self-contained feature record.
    pub packet_loss_ratio: f32,
    /// Aggregated band magnitudes (mean over subcarriers, normalized to a
    /// unit-depth cosine); one entry per in-band FFT bin.
    pub band_mags: Vec<f32>,
    pub band_bin_freqs: Vec<f64>,
    pub mover_count_estimate: Option<u64>,
    /// True when the emitted window bridged a sequence gap longer than
    /// `max_reconstruct_gap` (degraded-mode flag for fusion).
    pub long_gap_used: bool,
}

pub fn presence_from_energy(energy: f32, presence_ref: f32) -> f32 {
    if !energy.is_finite() || energy < 0.0 {
        return 0.0;
    }
    let e = energy.clamp(0.0, f32::MAX);
    e / (e + presence_ref.max(f32::EPSILON))
}

/// Pooled amplitude variance over the whole matrix (raw, pre-normalization).
pub fn motion_variance(amps: &[Vec<f32>]) -> f32 {
    let total: Vec<f32> = amps.iter().flat_map(|row| row.iter().copied()).collect();
    if total.is_empty() {
        return 0.0;
    }
    let mean = total.iter().sum::<f32>() / total.len() as f32;
    total
        .iter()
        .map(|v| {
            let d = v - mean;
            d * d
        })
        .sum::<f32>()
        / total.len() as f32
}

/// Count strict local maxima that stand out from the band's noise floor.
/// Used by both the feature path and tests; `None`-salary semantics stay in
/// the caller (WifiTick maps it to `Option<f64>`).
pub fn estimate_mover_count_from_band(band_mags: &[f32]) -> u64 {
    if band_mags.len() < 3 {
        return 0;
    }
    let med = crate::sanitize::median_f32(band_mags);
    let max = band_mags.iter().copied().fold(0.0f32, f32::max);
    let floor = med * 5.0;
    let mut peaks = 0u64;
    for i in 1..band_mags.len() - 1 {
        let m = band_mags[i];
        if m > band_mags[i - 1] && m > band_mags[i + 1] && m > floor && m > 0.5 * max {
            peaks += 1;
        }
    }
    peaks
}

/// Compute the spectrum-domain features for one sanitized window.
/// `packet_loss_ratio` and long-gap flags come from the sequence span (the
/// window processor owns them); this function is spectrum-only.
pub fn compute_window_features(
    window: &SanitizedWindow,
    cfg: &WindowConfig,
    packet_loss_ratio: f32,
    long_gap_used: bool,
) -> Result<WindowFeatures, CsiError> {
    cfg.validate()?;
    let t = window.packets();
    let s = window.subcarriers();
    if t < MIN_WINDOW_PACKETS || s == 0 {
        return Err(CsiError::InvalidConfig(format!(
            "window holds {} packets x {} subcarriers; needs >= {MIN_WINDOW_PACKETS} x 1",
            t, s
        )));
    }

    // AC-coupled series per subcarrier: subtract the time mean only.
    let mut rows: Vec<Vec<rustfft::num_complex::Complex<f32>>> = Vec::with_capacity(s);
    for k in 0..s {
        let mean = window.amps.iter().map(|row| row[k]).sum::<f32>() / t as f32;
        let row: Vec<_> = window
            .amps
            .iter()
            .map(|row| rustfft::num_complex::Complex::new(row[k] - mean, 0.0))
            .collect();
        rows.push(row);
    }

    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft(t, rustfft::FftDirection::Forward);
    let mut agg = vec![0f32; t];
    for row in &mut rows {
        fft.process(row);
        for (bin, value) in row.iter().enumerate() {
            agg[bin] += value.norm();
        }
    }
    // Mean over subcarriers, then normalize so a unit-depth cosine yields
    // magnitude ≈ 1 at its bin (|X| of depth·cos over t samples ≈ depth·t/2).
    let norm = (t as f32 / 2.0) * s as f32;
    for a in &mut agg {
        *a /= norm;
    }

    let mut band_mags = Vec::new();
    let mut band_bin_freqs = Vec::new();
    let mut energy = 0f32;
    for (bin, a) in agg.iter().enumerate() {
        let freq = bin as f64 * cfg.packet_rate_hz / t as f64;
        if freq >= cfg.doppler_min_hz && freq <= cfg.doppler_max_hz {
            band_mags.push(*a);
            band_bin_freqs.push(freq);
            energy += a;
        }
    }

    let variance = motion_variance(&window.amps);
    let presence = presence_from_energy(energy, cfg.presence_ref);
    let movers = estimate_mover_count_from_band(&band_mags);

    Ok(WindowFeatures {
        doppler_energy: energy,
        motion_variance: variance,
        presence_prob: presence,
        packet_loss_ratio,
        mover_count_estimate: if movers > 0 { Some(movers) } else { None },
        band_mags,
        band_bin_freqs,
        long_gap_used,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::record::DecodedPacket;
    use crate::sanitize::sanitize_packets;
    use common_types::EdgeStamp;

    fn stamp() -> EdgeStamp {
        EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, 0, 0, true)
    }

    fn tone_window(depth: f64, f_hz: f64, t: usize, s: usize, rate: f64) -> SanitizedWindow {
        let packets: Vec<DecodedPacket> = (0..t)
            .map(|i| {
                let phase = (std::f64::consts::TAU * f_hz * i as f64 / rate) as f32;
                let amp = 1.0f32 + depth as f32 * phase.cos();
                DecodedPacket {
                    seq: i as u64,
                    ts: stamp(),
                    amps: vec![amp; s],
                    phases: vec![0.0; s],
                }
            })
            .collect();
        sanitize_packets(&packets)
    }

    #[test]
    fn config_validation_rejects_degenerate_configs() {
        assert!(WindowConfig::defaults().validate().is_ok());
        let mut bad = WindowConfig::defaults();
        bad.window_packets = 4;
        assert!(bad.validate().is_err());
        let mut bad = WindowConfig::defaults();
        bad.stride_packets = 0;
        assert!(bad.validate().is_err());
        let mut bad = WindowConfig::defaults();
        bad.stride_packets = bad.window_packets + 1;
        assert!(bad.validate().is_err());
        let mut bad = WindowConfig::defaults();
        bad.packet_rate_hz = 0.0;
        assert!(bad.validate().is_err());
        let mut bad = WindowConfig::defaults();
        bad.doppler_min_hz = bad.doppler_max_hz;
        assert!(bad.validate().is_err());
        let mut bad = WindowConfig::defaults();
        bad.presence_ref = 0.0;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn walk_window_presence_is_high_and_finds_one_mover() {
        let cfg = WindowConfig::defaults();
        let window = tone_window(0.35, 8.0, 64, 8, 100.0);
        let feats = compute_window_features(&window, &cfg, 0.0, false).unwrap();
        assert!(
            feats.presence_prob > 0.8,
            "walk presence {} below floor",
            feats.presence_prob
        );
        assert!(feats.presence_prob < 0.999);
        assert_eq!(feats.mover_count_estimate, Some(1));
        assert!(
            feats.motion_variance > 0.04,
            "variance {}",
            feats.motion_variance
        );
        // The dominant spectral bin sits near the 8 Hz walker cadence.
        let peak_idx = feats
            .band_mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let peak_freq = feats.band_bin_freqs[peak_idx];
        assert!((peak_freq - 8.0).abs() < 3.0, "peak {peak_freq}");
    }

    #[test]
    fn two_movers_surface_two_peaks() {
        let cfg = WindowConfig::defaults();
        let t = 64usize;
        let packets: Vec<DecodedPacket> = (0..t)
            .map(|i| {
                let d1 = (std::f64::consts::TAU * 6.0 * i as f64 / 100.0).cos();
                let d2 = (std::f64::consts::TAU * 11.0 * i as f64 / 100.0).cos();
                DecodedPacket {
                    seq: i as u64,
                    ts: stamp(),
                    amps: vec![1.0 + 0.3 * (d1 as f32) + 0.3 * (d2 as f32); 8],
                    phases: vec![0.0; 8],
                }
            })
            .collect();
        let window = sanitize_packets(&packets);
        let feats = compute_window_features(&window, &cfg, 0.0, false).unwrap();
        assert_eq!(feats.mover_count_estimate, Some(2), "two distinct cadences");
    }

    #[test]
    fn idle_window_presence_is_low() {
        let cfg = WindowConfig::defaults();
        let window = tone_window(0.0001, 0.2, 64, 8, 100.0);
        let feats = compute_window_features(&window, &cfg, 0.0, false).unwrap();
        assert!(
            feats.presence_prob < 0.2,
            "idle presence {} too high",
            feats.presence_prob
        );
        assert!(feats.motion_variance < 1.0e-7);
    }

    #[test]
    fn motion_variance_walk_dominates_idle() {
        let walk = motion_variance(&tone_window(0.35, 8.0, 64, 8, 100.0).amps);
        let idle = motion_variance(&tone_window(0.0001, 0.2, 64, 8, 100.0).amps);
        assert!(walk > idle * 10.0, "walk {walk} vs idle {idle}");
    }

    #[test]
    fn short_windows_and_empty_reject() {
        let cfg = WindowConfig::defaults();
        let empty = SanitizedWindow {
            amps: Vec::new(),
            phases: Vec::new(),
            dropped_count: 0,
        };
        assert!(compute_window_features(&empty, &cfg, 0.0, false).is_err());
    }

    #[test]
    fn presence_mapping_is_monotonic() {
        assert!(presence_from_energy(0.0, 0.01) < 0.02);
        assert!(presence_from_energy(0.01, 0.01) < 0.51);
        assert!(presence_from_energy(0.1, 0.01) > 0.8);
        assert!(presence_from_energy(f32::NAN, 0.01) < 0.0001);
    }
}
