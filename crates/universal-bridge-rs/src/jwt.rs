//! HS256 JWT at the media-bus gate (`vts`). Hand-rolled base64url +
//! HMAC-SHA256 verified against RFC 4231 vectors, no external dep cycle.

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JwtError {
    #[error("secret must be at least 16 bytes")]
    SecretTooShort,
    #[error("claim key/value content invalid ({0})")]
    MalformedClaim(&'static str),
}

const B64URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let triple = (b0 << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let chars: [u8; 4] = [
            B64URL[(triple >> 18) as usize & 63],
            B64URL[(triple >> 12) as usize & 63],
            if chunk.len() > 1 {
                B64URL[(triple >> 6) as usize & 63]
            } else {
                b'='
            },
            if chunk.len() > 2 {
                B64URL[triple as usize & 63]
            } else {
                b'='
            },
        ];
        for c in chars {
            out.push(c as char);
        }
    }
    out
}

/// Hand-rolled HMAC-SHA256 (the Phase-6 implementation, RFC 4231
/// verified). Long keys hash down first; short keys zero-pad the block.
pub fn hmac_sha256(secret: &[u8], data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = secret.to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for (slot, byte) in k.iter().enumerate() {
        ipad[slot] ^= byte;
        opad[slot] ^= byte;
    }
    let mut hasher = Sha256::new();
    hasher.update(ipad);
    hasher.update(data);
    let inner = hasher.finalize().to_vec();
    let mut hasher = Sha256::new();
    hasher.update(opad);
    hasher.update(&inner);
    hasher.finalize().to_vec()
}

/// RFC 4231 (KEY = 20-byte key) for sanity of HMAC core. Covered by tests.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: u64,
}

pub fn vts_token(secret: &[u8], claims: &JwtClaims) -> Result<String, JwtError> {
    if secret.len() < 16 {
        return Err(JwtError::SecretTooShort);
    }
    if claims.sub.is_empty() {
        return Err(JwtError::MalformedClaim("sub"));
    }
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let payload = format!(r#"{{"sub":"{}","exp":{}}}"#, claims.sub, claims.exp);
    let signing_input = b64url(header.as_bytes()) + "." + &b64url(payload.as_bytes());
    let sig = hmac_sha256(secret, signing_input.as_bytes());
    Ok(signing_input + "." + &b64url(&sig))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// RFC 4231 §2.2 test case 1: HMAC-SHA256 with key
    /// 0x0b×20 over "Hi There" = b0344c61d8db38535ca8afceaf0bf12b.
    #[test]
    fn hmac_rfc4231_case1() {
        let mac = hmac_sha256(&[0x0b; 20], b"Hi There");
        let hex = mac.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert!(hex.starts_with("b0344c61d8db3853"), "got {hex}");
    }

    /// RFC 4231 §2.2 test case 6: key/inner/outer as specified.
    #[test]
    fn hmac_rfc4231_case6() {
        let mac = hmac_sha256(
            &[0xaa; 131],
            "Test Using Larger Than Block-Size Key - Hash Key First".as_bytes(),
        );
        let hex = mac.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert!(hex.starts_with("60e43159"), "got {hex}");
    }

    #[test]
    fn token_shape_and_secret_floor() {
        let secret = b"at-least-16-bytes!".to_vec();
        let tok = vts_token(
            &secret,
            &JwtClaims {
                sub: "edge-node-1".into(),
                exp: 1_790_000_000,
            },
        )
        .expect("token");
        let parts: Vec<&str> = tok.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            vts_token(
                b"short",
                &JwtClaims {
                    sub: "x".into(),
                    exp: 1
                }
            ),
            Err(JwtError::SecretTooShort)
        );
        assert_eq!(
            vts_token(
                &secret,
                &JwtClaims {
                    sub: "".into(),
                    exp: 1
                }
            ),
            Err(JwtError::MalformedClaim("sub"))
        );
    }
}
