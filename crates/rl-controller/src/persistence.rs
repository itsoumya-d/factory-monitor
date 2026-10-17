//! Filesystem persistence of the bandit's last-known-good state (docs/08 §8).
//! `data/rl/{zone_id}/{version}.rlw` is the long-term layout; the crate ships
//! a `SnapshotStore` the app layer points at that path per zone. pg `rl_audit`
//! rows are written by the api-server (Phase 6) using the same blob format.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tracing::info;

use crate::error::RlError;
use crate::policy::InternalState;

pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Atomic write: temp file + rename, so a crash never leaves a torn
    /// snapshot (the watchdog reads it on the next boot).
    pub fn save(&self, version: &str, state: &InternalState) -> Result<(), RlError> {
        let bytes =
            serde_json::to_vec(state).map_err(|err| RlError::Persistence(err.to_string()))?;
        let target = self.root.join(format!("{version}.rlw.json"));
        let tmp = self.root.join(format!("{version}.rlw.json.tmp"));
        fs::write(&tmp, &bytes).map_err(|err| RlError::Persistence(err.to_string()))?;
        fs::rename(&tmp, &target).map_err(|err| RlError::Persistence(err.to_string()))?;
        info!(path = %target.display(), bytes = bytes.len(), "RL snapshot saved");
        Ok(())
    }

    /// Latest snapshot by filename sort (lexicographic == version order for
    /// monotonic version tags); `None` when the store is empty (cold start →
    /// deterministic defaults, docs/08 §8).
    pub fn load_latest(&self) -> Option<InternalState> {
        let mut candidates: Vec<String> = fs::read_dir(&self.root)
            .ok()?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".rlw.json"))
            .collect();
        candidates.sort();
        let latest = candidates.pop()?;
        let bytes = fs::read(self.root.join(latest)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::policy::InternalState;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn snapshot(action: common_types::RlAction, frozen: bool) -> InternalState {
        InternalState {
            arms: vec![],
            last_safe: action,
            frozen,
            consecutive_vetoes: 0,
            pause_until: None,
            fp_bucket: 2,
            last_seen: None,
            history: Vec::new(),
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("shado-rl-{}-{}-{}", tag, std::process::id(), nanos))
    }

    #[test]
    fn save_then_load_latest_roundtrips_and_prefers_highest_version() {
        let root = temp_root("roundtrip");
        let store = SnapshotStore::open(&root).unwrap();
        assert!(store.load_latest().is_none(), "empty store is a cold start");

        let early = snapshot(common_types::RlAction::clamped(0.30, 0.35, 0.80, 20), false);
        store.save("0001", &early).unwrap();

        let late = snapshot(common_types::RlAction::clamped(0.60, 0.55, 0.30, 90), true);
        store.save("0002", &late).unwrap();

        let loaded = store.load_latest().unwrap();
        assert_eq!(
            loaded.last_safe, late.last_safe,
            "lexicographic pick = 0002"
        );
        assert!(loaded.frozen);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn save_is_atomic_no_temp_files_left_behind() {
        let root = temp_root("atomic");
        let store = SnapshotStore::open(&root).unwrap();
        store
            .save("v1", &snapshot(common_types::RlAction::defaults(), false))
            .unwrap();
        let leftovers: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().into_string().unwrap_or_default())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "rename consumed the temp file");
        std::fs::remove_dir_all(&root).ok();
    }
}
