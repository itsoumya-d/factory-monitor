//! CSI packet contract (docs/05 §2) and the two fixture-phase parsers.
//!
//! `Iq8` is the ESP32 CSI integer layout (interleaved signed 8-bit I/Q per
//! subcarrier). `IqFloatPcap` is the debug/pcap capture form (interleaved
//! f32-le I/Q). `Iq10IeeeIwl` needs the `iwlwifi` driver patch and lands with
//! the hardware phase — it parses as a hard `Unsupported` error so nothing
//! downstream can silently misread it.

use common_types::{EdgeStamp, ZoneId};
use serde::{Deserialize, Serialize};

use crate::error::CsiError;

/// ESP32 default capture slices the usable 20 MHz spectrum into 56 bins.
pub const ESP32_RECOMMENDED_SUBCARRIERS: u16 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsiFormat {
    /// Interleaved i8 [I, Q] pairs; amplitude in raw integer units.
    Iq8,
    /// Interleaved f32-le [I, Q] pairs from pcap captures.
    IqFloatPcap,
    /// `iwlwifi` patched-driver format (hardware phase, docs/15 §2 Phase 8).
    Iq10IeeeIwl,
}

impl CsiFormat {
    pub const fn payload_len(self, n_subcarriers: u16) -> usize {
        match self {
            Self::Iq8 => 2 * n_subcarriers as usize,
            Self::IqFloatPcap => 8 * n_subcarriers as usize,
            Self::Iq10IeeeIwl => 4 * n_subcarriers as usize,
        }
    }

    pub fn format_name(self) -> String {
        match self {
            Self::Iq8 => "Iq8".to_string(),
            Self::IqFloatPcap => "IqFloatPcap".to_string(),
            Self::Iq10IeeeIwl => "Iq10IeeeIwl".to_string(),
        }
    }
}

/// 48-bit IEEE MAC of the transmitting CSI node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> [u8; 6] {
        self.0
    }
}

impl std::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// One raw packet as delivered by any transport (docs/05 §2 header contract).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCsiRecord {
    pub node_id: u8,
    pub zone_hint: ZoneId,
    pub mac_src: MacAddr,
    pub n_subcarriers: u16,
    pub format: CsiFormat,
    pub seq_no: u64,
    pub t_recv: EdgeStamp,
    pub payload: Vec<u8>,
}

/// Amplitude/phase decomposition of one packet, post-parse.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPacket {
    pub seq: u64,
    pub ts: EdgeStamp,
    pub amps: Vec<f32>,
    pub phases: Vec<f32>,
}

impl RawCsiRecord {
    pub fn decode(&self) -> Result<DecodedPacket, CsiError> {
        let (amps, phases) = match self.format {
            CsiFormat::Iq8 => parse_iq8(&self.payload, self.n_subcarriers)?,
            CsiFormat::IqFloatPcap => parse_iq_f32le(&self.payload, self.n_subcarriers)?,
            CsiFormat::Iq10IeeeIwl => return Err(CsiError::Unsupported(self.format)),
        };
        Ok(DecodedPacket {
            seq: self.seq_no,
            ts: self.t_recv,
            amps,
            phases,
        })
    }
}

/// Parse the ESP32-style integer CSI layout. Amplitude stays in raw integer
/// units — every consumer only compares it against other values from the same
/// stream, and calibration happens at the fusion feature layer.
pub fn parse_iq8(payload: &[u8], n_subcarriers: u16) -> Result<(Vec<f32>, Vec<f32>), CsiError> {
    let expected = CsiFormat::Iq8.payload_len(n_subcarriers);
    if payload.len() != expected {
        return Err(CsiError::invalid_payload(
            CsiFormat::Iq8.format_name(),
            expected,
            payload.len(),
        ));
    }
    let mut amps = Vec::with_capacity(n_subcarriers as usize);
    let mut phases = Vec::with_capacity(n_subcarriers as usize);
    for pair in payload.chunks_exact(2) {
        let i = i8::from_le_bytes([pair[0]]);
        let q = i8::from_le_bytes([pair[1]]);
        let fi = f32::from(i);
        let fq = f32::from(q);
        amps.push((fi * fi + fq * fq).sqrt());
        phases.push(fq.atan2(fi));
    }
    Ok((amps, phases))
}

/// Parse the pcap float layout (f32-le, interleaved I/Q per subcarrier).
pub fn parse_iq_f32le(
    payload: &[u8],
    n_subcarriers: u16,
) -> Result<(Vec<f32>, Vec<f32>), CsiError> {
    let expected = CsiFormat::IqFloatPcap.payload_len(n_subcarriers);
    if payload.len() != expected {
        return Err(CsiError::invalid_payload(
            CsiFormat::IqFloatPcap.format_name(),
            expected,
            payload.len(),
        ));
    }
    let mut amps = Vec::with_capacity(n_subcarriers as usize);
    let mut phases = Vec::with_capacity(n_subcarriers as usize);
    for chunk in payload.chunks_exact(8) {
        let i = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let q = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        amps.push((i * i + q * q).sqrt());
        phases.push(q.atan2(i));
    }
    Ok((amps, phases))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use common_types::EdgeStamp;

    fn stamp() -> EdgeStamp {
        EdgeStamp::new(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH, 0, 0, true)
    }

    fn record(format: CsiFormat, n: u16, payload: Vec<u8>) -> RawCsiRecord {
        RawCsiRecord {
            node_id: 7,
            zone_hint: ZoneId::try_new("line_1").unwrap(),
            mac_src: MacAddr::new([0x30, 0xAE, 0xA4, 0x07, 0x1D, 7]),
            n_subcarriers: n,
            format,
            seq_no: 0,
            t_recv: stamp(),
            payload,
        }
    }

    #[test]
    fn iq8_amp_and_phase_decode() {
        let payload = vec![100u8, 0, 0, 100];
        let (amps, phases) = parse_iq8(&payload, 2).unwrap();
        assert_eq!(amps.len(), 2);
        assert!((amps[0] - 100.0).abs() < 1e-3, "I=100 Q=0 -> amp 100");
        assert!(phases[0].abs() < 1e-3);
        assert!((amps[1] - 100.0).abs() < 1e-3, "I=0 Q=100 -> amp 100");
        assert!((phases[1] - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn iq8_wrong_len_is_invalid_payload() {
        let err = parse_iq8(&[0u8; 3], 2).unwrap_err();
        assert!(matches!(err, CsiError::InvalidPayload { .. }));
        assert!(err.to_string().contains("needs 4"));
    }

    #[test]
    fn iqfloat_roundtrip() {
        let base = [1.0f32, -2.5f32, 0.75f32, 0.25f32];
        let payload = base
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>();
        // N is 2 subcarriers: pairs (1.0, -2.5), (0.75, 0.25).
        let (amps, phases) = parse_iq_f32le(&payload, 2).unwrap();
        assert!((amps[0] - (1.0f32 * 1.0 + 2.5 * 2.5).sqrt()).abs() < 1e-4);
        assert!((amps[1] - (0.75f32 * 0.75 + 0.25 * 0.25).sqrt()).abs() < 1e-4);
        assert!(phases.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn iq10_is_unsupported_until_hardware_phase() {
        let err = record(CsiFormat::Iq10IeeeIwl, 8, vec![0u8; 32])
            .decode()
            .unwrap_err();
        assert!(matches!(err, CsiError::Unsupported(CsiFormat::Iq10IeeeIwl)));
    }

    #[test]
    fn decode_iq8_dispatch() {
        let rec = record(CsiFormat::Iq8, 3, vec![10, 0, 0, 10, 10, 10]);
        let decoded = rec.decode().unwrap();
        assert_eq!(decoded.seq, 0);
        assert_eq!(decoded.amps.len(), 3);
        // I=Q=10: amp = sqrt(200), phase = pi/4.
        assert!((decoded.amps[2] - std::f32::consts::FRAC_PI_2 * 4.0).abs() > 1.0);
        assert!(((decoded.phases[2] - std::f32::consts::FRAC_PI_4).abs()) < 1e-4);
    }

    #[test]
    fn mac_display() {
        let mac = MacAddr::new([0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02]);
        assert_eq!(mac.to_string(), "de:ad:be:ef:01:02");
    }
}
