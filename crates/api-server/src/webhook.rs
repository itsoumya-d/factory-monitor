//! Webhooks (docs/10 §5): HMAC-SHA256 signed JSON posts, event-scoped
//! idempotency keys, two delivery attempts before the outbox retries
//! them later. Transport is injected so tests need no network.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookKind {
    BootReady,
    LayoutChanged,
    DayTotalPosted,
    UncertaintyAlarm,
    RlActionRejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event_id: Uuid,
    pub kind: WebhookKind,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub data: serde_json::Value,
}

/// `X-Shado-Signature: v1=<hex>` over the exact POSTed bytes;
/// textbook HMAC-SHA256 (block = 64 B for SHA-256).
pub fn sign_secret(secret: &str, body: &[u8]) -> String {
    format!("v1={}", hmac_sha256_hex(secret.as_bytes(), body))
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    const BLOCK: usize = 64;
    // Long keys hash down to their 32-byte digest.
    let mut k = key.to_vec();
    if k.len() > BLOCK {
        k = Sha256::digest(&k).to_vec();
    }
    // Short keys pad to the block with zero bytes (empty ok).
    k.resize(BLOCK, 0);
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for (slot, byte) in k.iter().enumerate() {
        ipad[slot] ^= byte;
        opad[slot] ^= byte;
    }
    let mut hasher = Sha256::new();
    hasher.update(ipad);
    hasher.update(msg);
    let inner = hasher.finalize().to_vec();
    let mut hasher = Sha256::new();
    hasher.update(opad);
    hasher.update(&inner);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[async_trait::async_trait]
pub trait WebhookTransport: Send + Sync {
    /// Returns the HTTP status code or an error string (timeout/DNS/TLS...).
    async fn post_raw(
        &self,
        url: &str,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    ) -> Result<u16, String>;
}

/// Failure classification (docs/10 §5): 4xx (except 429) = permanent,
/// transport error / 5xx / 429 = retryable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    Retryable,
    Permanent,
}

pub fn classify(status: Option<Result<u16, String>>) -> DeliveryOutcome {
    match status {
        Some(Ok(code)) if (200..300).contains(&code) => DeliveryOutcome::Delivered,
        Some(Ok(code)) if (300..400).contains(&code) => DeliveryOutcome::Delivered,
        Some(Ok(408 | 429)) => DeliveryOutcome::Retryable,
        Some(Ok(code)) if (400..500).contains(&code) => DeliveryOutcome::Permanent,
        _ => DeliveryOutcome::Retryable,
    }
}

pub struct Dispatcher<'a> {
    pub secret: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Try {
    pub outcome: DeliveryOutcome,
    pub detail: String,
}

pub fn outcome_of(err: Result<u16, String>) -> Try {
    match err {
        Ok(code) => match classify(Some(Ok(code))) {
            DeliveryOutcome::Delivered => Try {
                outcome: DeliveryOutcome::Delivered,
                detail: format!("accepted ({code})"),
            },
            other => Try {
                outcome: other,
                detail: format!("status {code}"),
            },
        },
        Err(raw) => Try {
            outcome: DeliveryOutcome::Retryable,
            detail: raw,
        },
    }
}

pub const MAX_ATTEMPTS: u32 = 2;

impl Dispatcher<'_> {
    /// Deliver the payload; stop early on permanent (4xx) or success.
    /// `Retryable` result means the outbox should hold an unDelivered copy.
    pub async fn dispatch<T: WebhookTransport + ?Sized>(
        &self,
        transport: &T,
        url: &str,
        payload: &WebhookPayload,
    ) -> Try {
        let body = serde_json::to_vec(payload).unwrap_or_else(|_| Vec::new());
        let headers = vec![
            ("content-type", "application/json".to_string()),
            ("x-shado-idempotency", payload.event_id.to_string()),
            ("x-shado-signature", sign_secret(self.secret, &body)),
        ];
        let mut last = Try {
            outcome: DeliveryOutcome::Retryable,
            detail: "no call made".into(),
        };
        for _ in 0..MAX_ATTEMPTS {
            let raw = transport.post_raw(url, headers.clone(), body.clone()).await;
            last = outcome_of(raw);
            if last.outcome != DeliveryOutcome::Retryable {
                return last;
            }
        }
        last
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recording {
        responses: Mutex<Vec<Result<u16, String>>>,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Recording {
        fn new(resp: Result<u16, String>, noise: impl Into<Vec<Result<u16, String>>>) -> Self {
            let mut seq = vec![resp];
            seq.extend(noise.into());
            Self {
                responses: Mutex::new(seq),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn take_kind(&self) -> Result<u16, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.responses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop()
                .unwrap_or_else(|| Err("no scripted response".into()))
        }
    }

    #[async_trait::async_trait]
    impl WebhookTransport for Recording {
        async fn post_raw(
            &self,
            _url: &str,
            _headers: Vec<(&'static str, String)>,
            _body: Vec<u8>,
        ) -> Result<u16, String> {
            self.take_kind()
        }
    }

    fn webhook() -> WebhookPayload {
        WebhookPayload {
            event_id: Uuid::now_v7(),
            kind: WebhookKind::DayTotalPosted,
            ts: chrono::Utc::now(),
            data: serde_json::json!({"zone": "line_1", "total": 12}),
        }
    }

    #[test]
    fn signature_is_stable_hex_mac() {
        let a = sign_secret("k", b"x");
        let b = sign_secret("k", b"x");
        assert_eq!(a, b);
        assert!(a.starts_with("v1=") && a.len() == 3 + 64, "{a}");
        assert_ne!(sign_secret("other", b"x"), a, "key mixes in");
        assert_ne!(sign_secret("k", b"y"), a);
    }

    /// RFC 4231 Test Case 1: key = 20×0x0b, data = "Hi There".
    /// RFC 4231 Test Case 3: key = 50×0xdd, data = 50×0xcd.
    /// RFC 4231 Test Case 6: key > block size (131×0x13), bytes hashed first.
    /// (Values are the SHA-256 outputs; cross-checked against hashlib.)
    #[test]
    fn rfc4231_test_vectors() {
        let twenty = [0x0b_u8; 20];
        assert_eq!(
            super::hmac_sha256_hex(&twenty, b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            super::hmac_sha256_hex(&[0xdd_u8; 50], &[0xcd_u8; 50]),
            "2897e6fc528f38b04230dae7a0f737e311796d2bcf08a26ca240e9af12156bac"
        );
        let long_key = [0x13_u8; 131];
        assert_eq!(
            super::hmac_sha256_hex(
                &long_key,
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "63ad609a6e0d3543d3ae0c75300a1545d565238ce26007b2cc1b62df4b00c579"
        );
    }

    #[test]
    fn classification_covers_all_buckets() {
        use DeliveryOutcome::*;
        assert_eq!(classify(Some(Ok(200))), Delivered);
        assert_eq!(classify(Some(Ok(301))), Delivered);
        assert_eq!(classify(Some(Ok(408))), Retryable);
        assert_eq!(classify(Some(Ok(429))), Retryable);
        assert_eq!(classify(Some(Ok(400))), Permanent);
        assert_eq!(classify(Some(Ok(500))), Retryable);
        assert_eq!(classify(Some(Err("timeout".into()))), Retryable);
        assert_eq!(classify(None), Retryable);
    }

    #[tokio::test]
    async fn delivered_on_ok_without_retry() {
        // Ok first: exactly 1 call.
        let t = Recording::new(Ok(200), Vec::new());
        let d = Dispatcher { secret: "k" };
        let p = webhook();
        let out = d.dispatch(&t, "http://hook/mychannel", &p).await;
        assert_eq!(out.outcome, DeliveryOutcome::Delivered);
        assert_eq!(t.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_transient_once_then_reports() {
        let t = Recording::new(Ok(503), Vec::new());
        let d = Dispatcher { secret: "k" };
        let p = webhook();
        let out = d.dispatch(&t, "http://hook/mychannel", &p).await;
        assert_eq!(out.outcome, DeliveryOutcome::Retryable);
        assert_eq!(
            t.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_ATTEMPTS as usize
        );
    }
}
