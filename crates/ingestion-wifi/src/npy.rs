//! Minimal Numpy `.npy` v1.0 serializer for f32 1-D arrays ('C'-order).
//!
//! Scope: read/write `'<f4'` vectors for CSI fixture dumps and debug tooling
//! (docs/05 §5 mentions NPY transcription). Anything else (f64, 2-D, Fortran
//! order, v2+ headers) is a hard `CorruptNpy` — feature creep goes through
//! fixtures, not silent coercion.

use std::io::{Read, Write};

use crate::error::CsiError;

const MAGIC: &[u8; 6] = b"\x93NUMPY";
const VERSION: [u8; 2] = [1, 0];

fn header_for(count: usize) -> String {
    let dict = format!(
        "{{'descr': '<f4', 'fortran_order': False, 'shape': ({count},), }}",
        count = count
    );
    // Pad with spaces so the total header (10 magic+version bytes + 2-byte
    // length + dict) is a multiple of 64, ending with a newline.
    let base = 6 + 1 + 1 + 2;
    let target = ((base + dict.len() + 1) / 64 + 1) * 64;
    let pad = target - base - dict.len() - 1;
    format!("{dict}{}\n", " ".repeat(pad))
}

/// Serialize one f32 slice to complete `.npy` bytes.
pub fn to_npy_f32_bytes(data: &[f32]) -> Vec<u8> {
    let header = header_for(data.len());
    let mut out = Vec::with_capacity(header.len() + 4 * data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION);
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    for value in data {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Deserialize complete `.npy` bytes into one f32 vector.
pub fn npy_f32_bytes_to_vec(bytes: &[u8]) -> Result<Vec<f32>, CsiError> {
    if bytes.len() < 10 || &bytes[..6] != MAGIC {
        return Err(CsiError::CorruptNpy("missing .npy magic".to_string()));
    }
    if bytes[6..8] != VERSION {
        return Err(CsiError::CorruptNpy(
            "only .npy v1.0 is supported".to_string(),
        ));
    }
    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    if bytes.len() < 10 + header_len {
        return Err(CsiError::CorruptNpy("header truncated".to_string()));
    }
    let header = std::str::from_utf8(&bytes[10..10 + header_len])
        .map_err(|e| CsiError::CorruptNpy(format!("header not UTF-8: {e}")))?;
    if !header.contains("'descr': '<f4'") {
        return Err(CsiError::CorruptNpy("descr must be '<f4'".to_string()));
    }
    if header.contains("'fortran_order': True") {
        return Err(CsiError::CorruptNpy(
            "fortran order unsupported".to_string(),
        ));
    }
    let shape_start = header
        .find("'shape': (")
        .ok_or_else(|| CsiError::CorruptNpy("shape missing".to_string()))?
        + "'shape': (".len();
    let shape_end = header[shape_start..]
        .find(')')
        .ok_or_else(|| CsiError::CorruptNpy("shape unterminated".to_string()))?
        + shape_start;
    let count_str = header[shape_start..shape_end].trim().trim_end_matches(',');
    let count: usize = count_str
        .parse()
        .map_err(|e| CsiError::CorruptNpy(format!("shape parse: {e}")))?;
    let data_start = 10 + header_len;
    let data_end = data_start + count * 4;
    if bytes.len() < data_end {
        return Err(CsiError::CorruptNpy("payload truncated".to_string()));
    }
    Ok(bytes[data_start..data_end]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// File-level convenience wrapper.
pub fn write_npy_f32(path: &std::path::Path, data: &[f32]) -> Result<(), CsiError> {
    let mut file =
        std::fs::File::create(path).map_err(|e| CsiError::CorruptNpy(format!("create: {e}")))?;
    file.write_all(&to_npy_f32_bytes(data))
        .map_err(|e| CsiError::CorruptNpy(format!("write: {e}")))?;
    Ok(())
}

/// File-level convenience wrapper.
pub fn read_npy_f32(path: &std::path::Path) -> Result<Vec<f32>, CsiError> {
    let mut file =
        std::fs::File::open(path).map_err(|e| CsiError::CorruptNpy(format!("open: {e}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| CsiError::CorruptNpy(format!("read: {e}")))?;
    npy_f32_bytes_to_vec(&bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn roundtrip() {
        let data = vec![1.0f32, -2.5, 3.25, 0.0, f32::MIN_POSITIVE];
        let bytes = to_npy_f32_bytes(&data);
        let back = npy_f32_bytes_to_vec(&bytes).unwrap();
        assert_eq!(back.len(), data.len());
        for (a, b) in back.iter().zip(data.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        // Header padding contract: payload starts on a 64-byte boundary.
        let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        assert_eq!((10 + header_len) % 64, 0);
    }

    #[test]
    fn empty_vector_roundtrip() {
        let bytes = to_npy_f32_bytes(&[]);
        assert_eq!(npy_f32_bytes_to_vec(&bytes).unwrap(), Vec::<f32>::new());
    }

    #[test]
    fn junk_bytes_rejected() {
        assert!(npy_f32_bytes_to_vec(&[0u8; 32]).is_err());
        let mut bytes = to_npy_f32_bytes(&[1.0f32]);
        bytes[6] = 2; // version 2.0 unsupported
        assert!(
            npy_f32_bytes_to_vec(&bytes)
                .unwrap_err()
                .to_string()
                .contains("v1.0")
        );
    }

    #[test]
    fn file_roundtrip_via_temp_dir() {
        let path = std::env::temp_dir().join(format!(
            "shado_npy_{}_{}.npy",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        write_npy_f32(&path, &[9.5f32, -0.25]).unwrap();
        let back = read_npy_f32(&path).unwrap();
        assert_eq!(back, vec![9.5f32, -0.25]);
        let _ = std::fs::remove_file(&path);
    }
}
