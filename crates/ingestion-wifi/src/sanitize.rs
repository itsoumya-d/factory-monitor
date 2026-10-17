//! Phase sanitization: the CSI pipeline stages [D]–[F] (docs/05 §2).
//!
//! * [D] MAD outlier filter — drop burst/static-interference packets whose
//!   mean amplitude deviates more than 3.5 robust sigmas, then rebuild those
//!   slots by neighbor reconstruction so every window stays dense.
//! * [E] per-packet subcarrier unwrap + mid-reference linear detrend —
//!   removes the conjugate-multiplication rotation (phase_vs_carrier ≈ linear
//!   in the subcarrier index).
//! * [F] temporal unwrap — per-subcarrier 2π-residue cleanup across packets.
//!
//! All functions are pure and deterministic (docs/14 contract).

use crate::record::DecodedPacket;

/// K-value of the MAD outlier rule (docs/05 §2 [D]).
pub const OUTLIER_SIGMA: f32 = 3.5;
/// Ratio scaling: MAD ≈ σ/1.4826 for clean Gaussian noise.
const MAD_TO_SIGMA: f32 = 1.482_6;

#[inline]
pub fn median_f32(values: &[f32]) -> f32 {
    let mut sorted: Vec<f32> = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.is_empty() {
        0.0
    } else if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

#[inline]
fn median_abs_dev(values: &[f32], med: f32) -> f32 {
    median_f32(&values.iter().map(|v| (v - med).abs()).collect::<Vec<_>>())
}

/// Dense, sanitized, time-ordered amplitude/phase matrices.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedWindow {
    /// `amps[t][s]` — amplitude matrix, packets × subcarriers.
    pub amps: Vec<Vec<f32>>,
    /// `phases[t][s]` — detrended, unwrapped phase matrix.
    pub phases: Vec<Vec<f32>>,
    /// Packets removed by the MAD filter then reconstructed.
    pub dropped_count: usize,
}

impl SanitizedWindow {
    pub fn packets(&self) -> usize {
        self.amps.len()
    }

    pub fn subcarriers(&self) -> usize {
        self.amps.first().map_or(0, Vec::len)
    }
}

/// Full sanitize pass in one call: outlier drop, neighbor reconstruction,
/// subcarrier unwrap + detrend, temporal unwrap.
pub fn sanitize_packets(packets: &[DecodedPacket]) -> SanitizedWindow {
    sanitize_packets_with(packets, OUTLIER_SIGMA)
}

/// Test-tunable variant (proptests use a relaxed sigma).
pub fn sanitize_packets_with(packets: &[DecodedPacket], outlier_sigma: f32) -> SanitizedWindow {
    let n = packets.len();
    let s = packets.first().map_or(0, |p| p.amps.len());
    if n == 0 || s == 0 {
        return SanitizedWindow {
            amps: Vec::new(),
            phases: Vec::new(),
            dropped_count: 0,
        };
    }

    // [D] mean-amplitude outliers.
    let scalars: Vec<f32> = packets
        .iter()
        .map(|p| p.amps.iter().sum::<f32>() / s as f32)
        .collect();
    let med = median_f32(&scalars);
    let mad = median_abs_dev(&scalars, med);
    let threshold = outlier_sigma * MAD_TO_SIGMA * mad;
    let dropped: Vec<bool> = scalars
        .iter()
        .map(|v| (v - med).abs() > threshold)
        .collect();

    // Rebuild dropped slots by nearest-kept-neighbor interpolation so the
    // window stays dense for the FFT stage. Leading gaps clamp to the first
    // kept packet, trailing gaps to the last one, an all-lost window to zero.
    let rebuild = |from: Option<usize>, to: Option<usize>, gap_t: usize| -> (Vec<f32>, Vec<f32>) {
        match (from, to) {
            (Some(a), Some(b)) => {
                let weight_b = (gap_t - a) as f32 / (b - a) as f32;
                (
                    interpolate(&packets[a].amps, &packets[b].amps, weight_b),
                    interpolate(&packets[a].phases, &packets[b].phases, weight_b),
                )
            }
            (Some(a), None) | (None, Some(a)) => {
                (packets[a].amps.clone(), packets[a].phases.clone())
            }
            (None, None) => (vec![0.0; s], vec![0.0; s]),
        }
    };
    let mut amps: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut phases: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut prev_kept: Option<usize> = None;
    let mut gap_slots: Vec<usize> = Vec::new();
    for (t, is_dropped) in dropped.iter().enumerate() {
        if !*is_dropped {
            for &gap_t in &gap_slots {
                let (amp, phase) = rebuild(prev_kept, Some(t), gap_t);
                amps.push(amp);
                phases.push(phase);
            }
            gap_slots.clear();
            prev_kept = Some(t);
            amps.push(packets[t].amps.clone());
            phases.push(packets[t].phases.clone());
        } else {
            gap_slots.push(t);
        }
    }
    for &gap_t in &gap_slots {
        let (amp, phase) = rebuild(prev_kept, None, gap_t);
        amps.push(amp);
        phases.push(phase);
    }

    let dropped_count = dropped.iter().filter(|d| **d).count();

    // [E] per-packet unwrap across subcarriers + mid-reference detrend.
    for phase in &mut phases {
        unwrap_and_detrend_across_subcarriers(phase);
    }

    // [F] temporal unwrap per subcarrier (in place over the matrix).
    phase_matrix_unwrap_over_time(&mut phases);

    SanitizedWindow {
        amps,
        phases,
        dropped_count,
    }
}

fn interpolate(a: &[f32], b: &[f32], weight_b: f32) -> Vec<f32> {
    a.iter()
        .zip(b.iter())
        .map(|(va, vb)| va * (1.0 - weight_b) + vb * weight_b)
        .collect()
}

/// Temporally unwrap a single subcarrier's phase sequence (2π residues).
pub fn unwrap_over_time(series: &mut [f32]) {
    for t in 1..series.len() {
        let delta = series[t] - series[t - 1];
        let correction = {
            let k = ((delta + std::f32::consts::PI) / (2.0 * std::f32::consts::PI)).floor();
            k * 2.0 * std::f32::consts::PI
        };
        series[t] -= correction;
    }
}

/// Unwrap the phase across the subcarrier axis of one packet, then remove the
/// linear carrier-wavefront by the mid-reference rule:
/// `phi'(k) = phi(k) - phi(mid) - slope * (k - mid)` where
/// `slope = (phi[N-1] - phi(mid)) / (N-1 - mid)`.
pub fn unwrap_and_detrend_across_subcarriers(phase: &mut [f32]) {
    for k in 1..phase.len() {
        let delta = phase[k] - phase[k - 1];
        let correction = {
            let m = ((delta + std::f32::consts::PI) / (2.0 * std::f32::consts::PI)).floor();
            m * 2.0 * std::f32::consts::PI
        };
        phase[k] -= correction;
    }
    let s = phase.len();
    if s >= 2 {
        let mid = (s - 1) / 2;
        let phi_mid = phase[mid];
        let tail = s - 1 - mid;
        let slope = (phase[s - 1] - phi_mid) / tail as f32;
        let mid_f = mid as f32;
        for (k, value) in phase.iter_mut().enumerate() {
            let kd = k as f32 - mid_f;
            *value -= phi_mid + slope * kd;
        }
    }
}

/// Unwrap a fixed button on mutated rows: temporal pass sits in the caller.
#[inline]
pub fn phase_matrix_unwrap_over_time(phases: &mut [Vec<f32>]) {
    let s = phases.first().map_or(0, Vec::len);
    for k in 0..s {
        let mut column: Vec<f32> = phases.iter().map(|row| row[k]).collect();
        unwrap_over_time(&mut column);
        for (t, value) in column.into_iter().enumerate() {
            phases[t][k] = value;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::record::DecodedPacket;
    use common_types::EdgeStamp;
    use proptest::prelude::*;

    fn stamp() -> EdgeStamp {
        EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, 0, 0, true)
    }

    fn packet(seq: u64, amps: Vec<f32>, phases: Vec<f32>) -> DecodedPacket {
        DecodedPacket {
            seq,
            ts: stamp(),
            amps,
            phases,
        }
    }

    #[test]
    fn constant_window_all_kept() {
        let packets: Vec<DecodedPacket> = (0..8)
            .map(|s| packet(s, vec![1.0; 4], vec![0.0; 4]))
            .collect();
        let window = sanitize_packets(&packets);
        assert_eq!(window.dropped_count, 0);
        assert_eq!(window.packets(), 8);
        assert_eq!(window.subcarriers(), 4);
    }

    #[test]
    fn outlier_dropped_and_neighbor_rebuilt() {
        let mut packets: Vec<DecodedPacket> = (0..9)
            .map(|s| packet(s, vec![1.0; 4], vec![0.0; 4]))
            .collect();
        packets[4] = packet(4, vec![9.0; 4], vec![0.0; 4]);
        let window = sanitize_packets(&packets);
        assert_eq!(window.dropped_count, 1);
        for value in &window.amps[4] {
            assert!((value - 1.0).abs() < 1e-4, "reconstruction fills {value}");
        }
    }

    #[test]
    fn linear_phase_detrends_to_zero() {
        let s = 16usize;
        let phases: Vec<f32> = (0..s).map(|k| 0.01 * k as f32).collect();
        let mut roundtrip = phases.clone();
        unwrap_and_detrend_across_subcarriers(&mut roundtrip);
        for value in &roundtrip {
            assert!(
                value.abs() < 1e-4,
                "linear wavefront must detrend to ~0, got {value}"
            );
        }
    }

    #[test]
    fn temporal_unwrap_removes_two_pi_jumps() {
        let packets = vec![
            packet(0, vec![1.0], vec![0.1]),
            packet(1, vec![1.0], vec![0.1 + std::f32::consts::TAU]),
            packet(2, vec![1.0], vec![0.15 + std::f32::consts::TAU]),
        ];
        let window = sanitize_packets(&packets);
        assert!((window.phases[1][0] - 0.1).abs() < 1e-3);
        assert!((window.phases[2][0] - 0.15).abs() < 1e-3);
    }

    proptest! {
        #[test]
        fn sanitize_preserves_packet_count(
            n in 8usize..64,
            s in 1usize..16,
        ) {
            let packets: Vec<DecodedPacket> = (0..n)
                .map(|t| {
                    let amps: Vec<f32> = (0..s)
                        .map(|_| 0.5 + 1.0 * std::f32::consts::PI * ((t * 7 + 3) as f32).sin().abs())
                        .collect();
                    let phases: Vec<f32> = (0..s)
                        .map(|k| (k as f32) * 0.1)
                        .collect();
                    packet(t as u64, amps, phases)
                })
                .collect();
            let window = sanitize_packets(&packets);
            prop_assert_eq!(window.packets(), n);
            prop_assert_eq!(window.subcarriers(), s);
            prop_assert!(window.dropped_count <= n);
        }
    }
}
