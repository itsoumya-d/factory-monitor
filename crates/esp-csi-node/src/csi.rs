//! LTF-strip parser + amplitude/phase extraction for ESP32 CSI packets.

#[cfg(not(test))]
use crate::Vec;
use thiserror::Error;

/// ESP32-C5/C6/S3 deliver up to 56 CSI subcarrier complexes per packet
/// (2 int8 components each). Legacy deployments in this repo use 52.
pub const MAX_SUBCARRIERS: usize = 56;
pub const MIN_SUBCARRIERS: usize = 16;

/// The first 4 bytes of every ESP32 CSI payload PHY framing garbage
/// (the L-LTF partial echo); docs and community drivers agree the first 4
/// bytes must be dropped before treating the rest as symmetric-complex
/// values. We can't validate the garbage contents — it varies — so we
/// enforce the *strip* and the remainder's shape.
pub const LTF_STRIP_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CsiError {
    #[error("payload shorter than the mandatory {LTF_STRIP_BYTES}-byte LTF strip")]
    TooShortForStrip,
    #[error(
        "remaining bytes ({0}) do not decompose into {MIN_SUBCARRIERS}..={MAX_SUBCARRIERS} subcarrier complexes of 2 bytes"
    )]
    NotSubcarrierPairs(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedCsi {
    pub re: Vec<i8>,
    pub im: Vec<i8>,
    /// Number of subcarriers = re.len() == im.len().
    pub n_subcarriers: usize,
}

/// Parse and strip one raw CSI payload (as delivered by the chip driver).
pub fn sanitize_frame(csi_payload: &[u8]) -> Result<SanitizedCsi, CsiError> {
    if csi_payload.len() <= LTF_STRIP_BYTES {
        return Err(CsiError::TooShortForStrip);
    }
    let rest = &csi_payload[LTF_STRIP_BYTES..];
    if rest.len() % 2 != 0 {
        return Err(CsiError::NotSubcarrierPairs(rest.len()));
    }
    let n = rest.len() / 2;
    if !(MIN_SUBCARRIERS..=MAX_SUBCARRIERS).contains(&n) {
        return Err(CsiError::NotSubcarrierPairs(rest.len()));
    }
    let mut re = Vec::with_capacity(n);
    let mut im = Vec::with_capacity(n);
    for pair in rest.chunks_exact(2) {
        re.push(pair[0] as i8);
        im.push(pair[1] as i8);
    }
    Ok(SanitizedCsi {
        re,
        im,
        n_subcarriers: n,
    })
}

/// |subcarrier| amplitude vector, zero-padded beyond n.
pub fn amplitudes(csi: &SanitizedCsi) -> Vec<f32> {
    csi.re
        .iter()
        .zip(csi.im.iter())
        .map(|(&r, &i)| {
            let rf = i32::from(r) as f32;
            let imf = i32::from(i) as f32;
            libm::sqrtf(rf * rf + imf * imf)
        })
        .collect()
}

/// Phase in radians via libm's no_std-safe atan2.
pub fn phases(csi: &SanitizedCsi) -> Vec<f32> {
    csi.re
        .iter()
        .zip(csi.im.iter())
        .map(|(&r, &i)| {
            let rf = i32::from(r) as f32;
            let imf = i32::from(i) as f32;
            libm::atan2f(imf, rf)
        })
        .collect()
}

/// Naive (no unwrap) phase difference in [-pi, pi].
pub fn phase_diff(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter()
        .zip(b.iter())
        .map(|(pa, pb)| {
            let d = pa - pb;
            let two_pi = 2.0 * core::f32::consts::PI;
            let mut d = d - two_pi * (d / two_pi).round();
            if d < -core::f32::consts::PI {
                d += two_pi;
            }
            if d > core::f32::consts::PI {
                d -= two_pi;
            }
            d
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn synthetic_payload(n_subcarriers: usize) -> Vec<u8> {
        let mut v = vec![0u8; LTF_STRIP_BYTES + 2 * n_subcarriers];
        for i in 0..n_subcarriers {
            v[LTF_STRIP_BYTES + 2 * i] = (i % 31) as u8;
            v[LTF_STRIP_BYTES + 2 * i + 1] = ((i * 3 + 7) % 31) as u8;
        }
        v
    }

    #[test]
    fn strip_is_mandatory_and_shape_checked() {
        let payload = synthetic_payload(52);
        let csi = sanitize_frame(&payload).expect("52-subcarrier payload is standard");
        assert_eq!(csi.n_subcarriers, 52);
        assert_eq!(csi.re.len(), 52);
        assert_eq!(csi.im.len(), 52);

        // The strip moves every subcarrier exactly 4 bytes: pair 0 of the
        // payload becomes (payload[4], payload[5]).
        assert_eq!(csi.re[0], payload[4] as i8);
        assert_eq!(csi.im[0], payload[5] as i8);

        assert_eq!(
            sanitize_frame(&payload[..4]),
            Err(CsiError::TooShortForStrip)
        );
        let odd = synthetic_payload(40);
        assert_eq!(
            sanitize_frame(&odd[..LTF_STRIP_BYTES + 3]),
            Err(CsiError::NotSubcarrierPairs(3))
        );
        let too_many = synthetic_payload(60);
        assert!(sanitize_frame(&too_many).is_err());
    }

    #[test]
    fn amplitude_pythagoras_verified() {
        // Subcarrier (3, 4) → amplitude 5.0; 16-pair payload (MIN limit).
        let mut payload = vec![0u8; LTF_STRIP_BYTES + 32];
        payload[LTF_STRIP_BYTES] = 3;
        payload[LTF_STRIP_BYTES + 1] = 4;
        let csi = sanitize_frame(&payload).expect("16 subcarriers");
        let amps = amplitudes(&csi);
        assert!((amps[0] - 5.0).abs() < 1e-6, "pythagoras: {}", amps[0]);
        assert_eq!(amps[1], 0.0);
    }

    #[test]
    fn phases_are_bounded() {
        let payload = synthetic_payload(32);
        let csi = sanitize_frame(&payload).expect("32 subcarriers");
        for p in phases(&csi) {
            assert!((-core::f32::consts::PI..=core::f32::consts::PI).contains(&p));
        }
    }

    #[test]
    fn phase_diff_wraps_into_pi_range() {
        // a=0.0, b=5.5 rad: raw diff −5.5 wraps to 2π−5.5 ≈ 0.78318.
        let a = vec![0.0];
        let b = vec![5.5];
        let d = phase_diff(&a, &b);
        assert!(
            d.iter()
                .all(|v| (-core::f32::consts::PI..=core::f32::consts::PI).contains(v))
        );
        assert!((d[0] - 0.78318).abs() < 1e-4, "wrapped value: {}", d[0]);
    }
}
