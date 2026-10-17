//! Egress policy: where resolved events go.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookChannel {
    Http,
    Slack,
    Teams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    Dwell,
    ZoneEntry,
    ZoneExit,
    LineCross,
    CountSpike,
    CsiAlert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u8,
    pub backoff_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookBinding {
    pub channel: WebhookChannel,
    pub url: String,
    pub secret_ref: Option<String>,
    pub events: Vec<WebhookEvent>,
    pub retry: RetryPolicy,
}

impl WebhookBinding {
    /// Only https egress is accepted — this is a locked schema rule so an
    /// on-prem proxy must terminate TLS or the template author fixes it.
    pub fn validate(&self) -> Result<(), WebhookError> {
        if !self.url.starts_with("https://") {
            return Err(WebhookError::InsecureUrl);
        }
        if self.events.is_empty() {
            return Err(WebhookError::NoEvents);
        }
        if self.retry.max_attempts == 0 || self.retry.max_attempts > 8 {
            return Err(WebhookError::RetryPolicyOutOfRange);
        }
        if self.retry.backoff_ms == 0 {
            return Err(WebhookError::RetryPolicyOutOfRange);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebhookError {
    #[error("https-only egress")]
    InsecureUrl,
    #[error("binding subscribes to no events")]
    NoEvents,
    #[error("retry attempts in [1, 8] and backoff > 0")]
    RetryPolicyOutOfRange,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn binding(url: &str) -> WebhookBinding {
        WebhookBinding {
            channel: WebhookChannel::Slack,
            url: url.to_string(),
            secret_ref: Some("vault://hooks/factory".into()),
            events: vec![WebhookEvent::Dwell],
            retry: RetryPolicy {
                max_attempts: 3,
                backoff_ms: 5_000,
            },
        }
    }

    #[test]
    fn https_enforced() {
        assert!(binding("https://hooks.example/shado").validate().is_ok());
        assert!(binding("http://hooks.example/shado").validate().is_err());
    }

    #[test]
    fn retry_bounds() {
        let mut b = binding("https://h.x");
        b.retry = RetryPolicy {
            max_attempts: 9,
            backoff_ms: 1,
        };
        assert!(b.validate().is_err());
    }
}
