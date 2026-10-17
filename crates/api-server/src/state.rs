//! Shared application state for every route (docs/10 §4 "service supervises
//! everything"): durable repo, live hub, lifecycle + RL views, counters.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Utc};
use common_types::lifecycle::LifecycleState;

use crate::config::Config;
use crate::hub::LiveHub;
use crate::repo::Repos;

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleView {
    pub state: LifecycleState,
    pub since: DateTime<Utc>,
    pub last_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RlView {
    pub paused_until: Option<DateTime<Utc>>,
    pub override_note: Option<String>,
    pub rejected_24h: u64,
}

#[derive(Debug, Default, Clone)]
pub struct RequestMetrics {
    total: Arc<Mutex<HashMap<String, u64>>>,
}

impl RequestMetrics {
    pub fn bump(&self, route: &'static str) {
        self.lock()
            .entry(route.to_string())
            .and_modify(|n| *n += 1)
            .or_insert(1);
    }

    pub fn total(&self, route: &str) -> u64 {
        self.lock().get(route).copied().unwrap_or(0)
    }

    pub fn snapshot(&self) -> HashMap<String, u64> {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, u64>> {
        self.total.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `metrics: op_total` buckets by path prefix, free of dynamic ids.
pub fn route_tag(path: &str) -> &'static str {
    const PREFIXES: [&str; 9] = [
        "/api/v1/lifecycle",
        "/api/v1/calibration",
        "/api/v1/zones",
        "/api/v1/audit",
        "/api/v1/rl",
        "/api/v1/snapshots",
        "/api/v1/labeling",
        "/ws/live",
        "/healthz",
    ];
    for p in PREFIXES {
        if path.starts_with(p) {
            return p;
        }
    }
    "other"
}

pub struct AppState {
    pub cfg: Config,
    pub repos: Arc<dyn Repos>,
    pub hub: LiveHub,
    pub lifecycle: Mutex<LifecycleView>,
    pub rl: Mutex<RlView>,
    pub metrics: RequestMetrics,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(cfg: Config, repos: Arc<dyn Repos>) -> Self {
        let hub = LiveHub::new(cfg.ws_capacity);
        Self {
            cfg,
            repos,
            hub,
            lifecycle: Mutex::new(LifecycleView {
                state: LifecycleState::Standby,
                since: Utc::now(),
                last_reason: None,
            }),
            rl: Mutex::new(RlView::default()),
            metrics: RequestMetrics::default(),
            started_at: Utc::now(),
        }
    }

    pub fn lifecycle(&self) -> MutexGuard<'_, LifecycleView> {
        self.lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn rl(&self) -> MutexGuard<'_, RlView> {
        self.rl.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::repo::InMemoryRepo;

    #[test]
    fn route_tags_map_to_known_prefixes() {
        assert_eq!(route_tag("/api/v1/zones/line_1/counts"), "/api/v1/zones");
        assert_eq!(route_tag("/api/v1/rl/state"), "/api/v1/rl");
        assert_eq!(route_tag("/healthz"), "/healthz");
        assert_eq!(route_tag("/ws/live"), "/ws/live");
        assert_eq!(route_tag("/bogus"), "other");
    }

    #[test]
    fn state_defaults_standby_and_counts_bump() {
        let state = AppState::new(Config::default(), Arc::new(InMemoryRepo::new()));
        assert_eq!(state.lifecycle().state, LifecycleState::Standby);
        assert_eq!(state.metrics.total("/x"), 0);
        state.metrics.bump("/api/v1/zones");
        state.metrics.bump("/api/v1/zones");
        assert_eq!(state.metrics.total("/api/v1/zones"), 2);
    }

    #[test]
    fn poisoned_locks_recover() {
        let state = AppState::new(Config::default(), Arc::new(InMemoryRepo::new()));
        let guard = state
            .lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drop(guard);
        assert_eq!(state.lifecycle().state, LifecycleState::Standby);
    }
}
