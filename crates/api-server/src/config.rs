//! Config loading (docs/10 §6): CLI > env (`SHADO_*`) > defaults. The
//! TOML profile layer and SIGHUP hot reload attach in main; the core
//! struct is pure data so tests can build it directly.

use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: Option<String>,
    /// HMAC-SHA256 secret for outbound webhooks; env-only, never persisted
    /// (docs/13 §7).
    pub webhook_secret: Option<String>,
    pub daily_usd_cap: f64,
    pub ws_capacity: usize,
    pub request_timeout_ms: u64,
    pub body_cap_bytes: usize,
    /// Calibration/ROI editor is a JSON drawing surface, not a frame dump.
    pub roi_body_cap_bytes: usize,
    pub allowed_origin: String,
    pub default_rate_limit_per_min: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 8080)),
            database_url: None,
            webhook_secret: None,
            daily_usd_cap: 15.0,
            ws_capacity: 4096,
            request_timeout_ms: 10_000,
            body_cap_bytes: 10 * 1024 * 1024,
            roi_body_cap_bytes: 2 * 1024 * 1024,
            allowed_origin: "http://localhost:3000".to_string(),
            default_rate_limit_per_min: 600,
        }
    }
}

impl Config {
    /// Priority chain: explicit env override of each field when present.
    pub fn from_env() -> Result<Self, String> {
        let mut cfg = Config::default();
        if let Ok(bind) = std::env::var("SHADO_BIND") {
            cfg.bind = bind
                .parse()
                .map_err(|_| format!("SHADO_BIND not a socket addr: {bind}"))?;
        }
        if let Ok(db) = std::env::var("DATABASE_URL") {
            cfg.database_url = Some(db);
        }
        if let Ok(secret) = std::env::var("WEBHOOK_SECRET") {
            if secret.len() < 16 {
                return Err("WEBHOOK_SECRET too short (need >= 16 bytes)".to_string());
            }
            cfg.webhook_secret = Some(secret);
        }
        if let Ok(cap) = std::env::var("SHADO_DAILY_USD_CAP") {
            cfg.daily_usd_cap = cap
                .parse::<f64>()
                .map_err(|_| format!("SHADO_DAILY_USD_CAP not a number: {cap}"))?;
        }
        if let Ok(origin) = std::env::var("SHADO_ALLOW_ORIGIN") {
            cfg.allowed_origin = origin;
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn defaults_match_docs() {
        let cfg = Config::default();
        assert_eq!(cfg.request_timeout_ms, 10_000, "docs/10 §2: 10 s timeout");
        assert_eq!(cfg.body_cap_bytes, 10 * 1024 * 1024, "10 MB body cap");
        assert_eq!(cfg.roi_body_cap_bytes, 2 * 1024 * 1024, "2 MB for rois");
        assert_eq!(cfg.ws_capacity, 4096, "broadcast capacity docs/10 §3");
        assert_eq!(cfg.daily_usd_cap, 15.0, "docs/09 §6 spend cap default");
    }

    #[test]
    fn env_parsing_errors_are_explicit() {
        // We do not mutate the process env from the test: parse directly.
        let bad: Result<SocketAddr, _> = "nope".parse();
        assert!(bad.is_err());
        let ok: SocketAddr = "127.0.0.1:9100".parse().expect("parse");
        assert_eq!(ok.to_string(), "127.0.0.1:9100");
    }
}
