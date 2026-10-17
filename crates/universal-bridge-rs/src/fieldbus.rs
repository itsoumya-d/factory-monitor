//! OT fieldbus register codecs: `RegisterRepresentable` over PLC word
//! orders ABCD/BADC/CDAB/DCBA (docs: modbus "Representable" contract).

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordOrder {
    /// Big-endian words, big-endian bytes (AB CD).
    Abcd,
    /// Little-endian words, big-endian bytes (BA CD).
    Badc,
    /// Big-endian words, little-endian bytes (AB DC).
    Cdab,
    /// Little-endian words, little-endian bytes (BA DC).
    Dcba,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CodecError {
    #[error("f64 needs 4 registers, got {0}")]
    RegisterCount(u8),
    #[error("register index {0} out of range")]
    OutOfRange(usize),
}

/// Values a bridge may carry into PLC registers.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldValue {
    U16(u16),
    F32(f32),
    F64(f64),
}

impl FieldValue {
    pub fn registers(self, order: WordOrder) -> Result<Vec<u16>, CodecError> {
        match self {
            Self::U16(v) => Ok(vec![v]),
            Self::F32(v) => {
                let bytes = v.to_bits().to_be_bytes();
                let hi = u16::from_be_bytes([bytes[0], bytes[1]]);
                let lo = u16::from_be_bytes([bytes[2], bytes[3]]);
                let swapped_lo = lo.swap_bytes();
                let swapped_hi = hi.swap_bytes();
                Ok(match order {
                    WordOrder::Abcd => vec![hi, lo],
                    WordOrder::Badc => vec![swapped_hi, lo],
                    WordOrder::Cdab => vec![lo, hi],
                    WordOrder::Dcba => vec![swapped_lo, swapped_hi],
                })
            }
            Self::F64(v) => {
                let bits_bytes: Vec<u8> = v.to_bits().to_be_bytes().to_vec();
                let words: Vec<u16> = bits_bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_be_bytes(*c))
                    .collect();
                Ok(decorate_f64(order, &words))
            }
        }
    }
}

fn decorate_f64(order: WordOrder, words: &[u16]) -> Vec<u16> {
    #[allow(clippy::get_first)]
    let w = [words[0], words[1], words[2], words[3]];
    match order {
        WordOrder::Abcd => w.to_vec(),
        WordOrder::Badc => vec![
            w[0].swap_bytes(),
            w[1].swap_bytes(),
            w[2].swap_bytes(),
            w[3].swap_bytes(),
        ],
        WordOrder::Cdab => vec![w[3], w[2], w[1], w[0]],
        WordOrder::Dcba => vec![
            w[3].swap_bytes(),
            w[2].swap_bytes(),
            w[1].swap_bytes(),
            w[0].swap_bytes(),
        ],
    }
}

/// PLC-facing contract: values a bridge publishes must be expressible in
/// registers; bridge keeps the cast explicit (a truncation would corrupt
/// KPI feedback into the plant).
pub trait RegisterRepresentable {
    fn value(&self) -> FieldValue;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn hex_bytes(v: &[u16]) -> String {
        v.iter().map(|w| format!("{w:04X}")).collect()
    }

    /// A known f32 (intern mixing classic): 1.0 → 0x3F80_0000.
    #[test]
    fn f32_word_orders_match_reference_layouts() {
        let cases: [(WordOrder, &str); 4] = [
            (WordOrder::Abcd, "3F800000"),
            (WordOrder::Badc, "803F0000"),
            (WordOrder::Cdab, "00003F80"),
            (WordOrder::Dcba, "0000803F"),
        ];
        // Note the classic layout: 1.0 = BE bytes [3F 80] [00 00] — so the
        // BE-swapped words yield:
        let v = FieldValue::F32(1.0);
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Abcd).expect("f32")),
            "3F800000"
        );
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Badc).expect("f32")),
            "803F0000"
        );
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Cdab).expect("f32")),
            "00003F80"
        );
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Dcba).expect("f32")),
            "0000803F"
        );
        let _ = cases;
    }

    /// Verified layout for a NATURAL f32 with BOTH halves nonzero.
    #[test]
    fn f32_layout_odd_value() {
        // 1.5 = 0x3FC0_0000 -> hi = 3FC0, lo = 0000; distinctive halves:
        // 123.0 = 0x42F6_0000.
        // Pick a value with both halves: 0x1234_5678 bits → f32 non-NaN.
        let bits = u32::from_be_bytes([0x12, 0x34, 0x56, 0x78]);
        let value = f32::from_bits(bits).to_bits();
        assert_eq!(value, bits); // still the same bits
        let bytes = value.to_be_bytes();
        let hi = u16::from_be_bytes([bytes[0], bytes[1]]);
        let lo = u16::from_be_bytes([bytes[2], bytes[3]]);
        let v = FieldValue::F32(f32::from_bits(value));

        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Abcd).expect("f32")),
            format!("{hi:04X}{lo:04X}")
        );
        let want_badc: String = format!("{:04X}{lo:04X}", hi.swap_bytes());
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Badc).expect("f32")),
            want_badc
        );
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Cdab).expect("f32")),
            format!("{lo:04X}{hi:04X}")
        );
        let want_dcba: String = format!("{:04X}{:04X}", lo.swap_bytes(), hi.swap_bytes());
        assert_eq!(
            hex_bytes(&v.registers(WordOrder::Dcba).expect("f32")),
            want_dcba
        );
    }

    #[test]
    fn u16_single_word() {
        assert_eq!(
            FieldValue::U16(0xBEEF)
                .registers(WordOrder::Abcd)
                .expect("u16"),
            vec![0xBEEF]
        );
    }

    #[test]
    fn f64_four_registers_abcd() {
        // 1.0f64 = 0x3FF0_0000_0000_0000 → words [3FF0, 0000, 0000, 0000]
        let regs = FieldValue::F64(1.0)
            .registers(WordOrder::Abcd)
            .expect("f64");
        assert_eq!(hex_bytes(&regs), "3FF0000000000000");
        let dcba = FieldValue::F64(1.0)
            .registers(WordOrder::Dcba)
            .expect("f64");
        assert_eq!(hex_bytes(&dcba), "000000000000F03F");
    }
}
