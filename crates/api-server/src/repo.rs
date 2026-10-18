//! Storage contract for the api-server (docs/10 + docs/11).
//!
//! Routes only depend on the [`Repos`] trait, so unit tests run against
//! [`InMemoryRepo`] with zero database; the PostgreSQL implementation
//! behind `--features itest` swaps in at boot via `main`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Datelike, Utc};
use common_types::counting::PackageCountEvent;
use common_types::feedback::HumanAuditFeedback;
use common_types::ids::{CameraId, OperatorId, ZoneId};
use common_types::lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// 2D calibration point in image pixel space.
pub type CalibPoint = (f32, f32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoiLine {
    #[serde(with = "point_list")]
    pub points: Vec<CalibPoint>,
}

impl RoiLine {
    pub fn from_pairs(points: Vec<CalibPoint>) -> ApiResult<Self> {
        if points.len() < 2 {
            return Err(ApiError::BadRequest(
                "an ROI needs at least two points".to_string(),
            ));
        }
        Ok(Self { points })
    }
}

mod point_list {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::CalibPoint;

    pub fn serialize<S: Serializer>(ps: &[CalibPoint], s: S) -> Result<S::Ok, S::Error> {
        ps.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<CalibPoint>, D::Error> {
        Vec::<CalibPoint>::deserialize(d)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub zone_id: ZoneId,
    pub display_name: String,
    pub camera_ids: Vec<CameraId>,
    pub csi_node_ids: Vec<u16>,
    pub roi_lines: Vec<RoiLine>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Zone {
    pub fn try_new(zone_id: ZoneId, display_name: String) -> ApiResult<Self> {
        if display_name.trim().is_empty() {
            return Err(ApiError::BadRequest("display_name required".to_string()));
        }
        Ok(Self {
            zone_id,
            display_name,
            camera_ids: Vec::new(),
            csi_node_ids: Vec::new(),
            roi_lines: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }
}

/// One row of `calibration_state` (docs/11 §1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSnapshotRow {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub lifecycle: LifecycleState,
    pub reason: Option<String>,
    pub vlm_diff: Option<Value>,
    pub roi_changes: Option<Value>,
    pub cameras_ok: Option<u16>,
    pub cameras_total: Option<u16>,
    pub csi_nodes_ok: Option<u16>,
    pub csi_nodes_total: Option<u16>,
}

/// `audit_corrections.outcome` (docs/11 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionOutcome {
    Submitted,
    Committed,
    Superseded,
}

impl CorrectionOutcome {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Committed => "committed",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse_db(raw: &str) -> Option<Self> {
        match raw {
            "submitted" => Some(Self::Submitted),
            "committed" => Some(Self::Committed),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditCorrectionRow {
    pub correction_id: Uuid,
    pub feedback: HumanAuditFeedback,
    pub outcome: CorrectionOutcome,
    pub reward_computed: Option<f32>,
    pub root_cause_report: Option<Value>,
    pub created_at: DateTime<Utc>,
}

/// `rl_audit.reason` vocabulary (docs/11 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RlReason {
    EodCorrection,
    IntradayShape,
    GuardrailReject,
    ManualOverride,
    Restore,
}

impl RlReason {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::EodCorrection => "eod_correction",
            Self::IntradayShape => "intraday_shape",
            Self::GuardrailReject => "guardrail_reject",
            Self::ManualOverride => "manual_override",
            Self::Restore => "restore",
        }
    }

    pub fn parse_db(raw: &str) -> Option<Self> {
        Some(match raw {
            "eod_correction" => Self::EodCorrection,
            "intraday_shape" => Self::IntradayShape,
            "guardrail_reject" => Self::GuardrailReject,
            "manual_override" => Self::ManualOverride,
            "restore" => Self::Restore,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RlTransitionRow {
    pub transition_id: Uuid,
    pub ts: DateTime<Utc>,
    pub zone_id: Option<ZoneId>,
    pub before: common_types::action::RlAction,
    pub after: common_types::action::RlAction,
    pub reward: Option<f32>,
    pub was_clamped: bool,
    pub rejected: bool,
    pub reason: RlReason,
}

/// `uncertain_snapshots.promotion` latch states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum SnapshotPromotion {
    Amber,
    Uploaded,
    AutoLabeled,
    LabeledOk,
    Trained,
    /// Operator-determined: bad frame (blur, wrong zone, duplicate) —
    /// leaves the label queue and never becomes training data.
    Rejected,
}

impl SnapshotPromotion {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Amber => "Amber",
            Self::Uploaded => "Uploaded",
            Self::AutoLabeled => "AutoLabeled",
            Self::LabeledOk => "LabeledOK",
            Self::Trained => "Trained",
            Self::Rejected => "Rejected",
        }
    }

    pub fn parse_db(raw: &str) -> Option<Self> {
        Some(match raw {
            "Amber" => Self::Amber,
            "Uploaded" => Self::Uploaded,
            "AutoLabeled" => Self::AutoLabeled,
            "LabeledOK" => Self::LabeledOk,
            "Trained" => Self::Trained,
            "Rejected" => Self::Rejected,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRow {
    pub frame_id: Uuid,
    pub zone_id: ZoneId,
    pub ts: DateTime<Utc>,
    pub camera_id: CameraId,
    /// Always stripped before the search route answers (priv):
    /// only `/snapshots/image/{frame_id}` may hand bytes to an operator.
    #[serde(skip)]
    pub jpeg_lowres: Option<Vec<u8>>,
    pub uncertainty_mask: u16,
    pub divergence: Option<f32>,
    pub state: Value,
    pub promotion: SnapshotPromotion,
    pub vlm_root_cause: Option<String>,
    pub coco_annotations: Option<Value>,
    pub reviewed_by: Option<OperatorId>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookStatus {
    Pending,
    Delivered,
    Failed,
    Dead,
}

impl WebhookStatus {
    pub const fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Dead => "dead",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxItem {
    pub event_id: Uuid,
    pub channel: String,
    pub url: String,
    pub payload: Value,
    pub status: WebhookStatus,
    pub attempts: u16,
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Detector,
    Csi,
    Rl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    Staged,
    Shadow,
    Active,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRow {
    pub version: common_types::ids::ModelVersion,
    pub kind: ModelKind,
    pub path: String,
    pub checksum: String,
    pub size_bytes: Option<i64>,
    pub train_dataset_ref: Option<String>,
    pub eval_report: Option<Value>,
    pub status: ModelStatus,
    pub activated_at: Option<DateTime<Utc>>,
}

/// Exactly one backend (memory / pg) behind every route.
#[async_trait::async_trait]
pub trait Repos: Send + Sync {
    async fn put_zone(&self, zone: Zone) -> Result<(), ApiError>;
    async fn get_zone(&self, zone_id: &ZoneId) -> Result<Option<Zone>, ApiError>;
    async fn list_zones(&self) -> Result<Vec<Zone>, ApiError>;
    async fn retire_zone(&self, zone_id: &ZoneId) -> ApiResult<()>;

    async fn put_calibration(&self, snapshot: CalibrationSnapshotRow) -> ApiResult<()>;
    async fn latest_calibration(&self) -> Result<Option<CalibrationSnapshotRow>, ApiError>;

    async fn insert_count_event(&self, event: PackageCountEvent) -> ApiResult<()>;
    async fn count_events(
        &self,
        zone_id: &ZoneId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<PackageCountEvent>, ApiError>;

    async fn put_correction(&self, row: AuditCorrectionRow) -> ApiResult<()>;
    async fn get_correction(&self, id: Uuid) -> Result<Option<AuditCorrectionRow>, ApiError>;
    async fn set_correction_outcome(&self, id: Uuid, outcome: CorrectionOutcome) -> ApiResult<()>;
    /// Submitted corrections in `ts_from` order.
    async fn pending_corrections(&self) -> Result<Vec<AuditCorrectionRow>, ApiError>;

    async fn put_rl_transition(&self, row: RlTransitionRow) -> ApiResult<()>;
    /// All transitions with `ts` on `date` (UTC calendar day), newest first.
    async fn rl_history(&self, date_ymd: (i32, u32, u32))
    -> Result<Vec<RlTransitionRow>, ApiError>;

    async fn put_snapshot(&self, row: SnapshotRow) -> ApiResult<()>;
    async fn get_snapshot(&self, frame_id: Uuid) -> Result<Option<SnapshotRow>, ApiError>;
    async fn set_snapshot_promotion(
        &self,
        frame_id: Uuid,
        promotion: SnapshotPromotion,
    ) -> ApiResult<()>;
    async fn review_snapshot(
        &self,
        frame_id: Uuid,
        reviewed_by: OperatorId,
        coco_annotations: Option<Value>,
    ) -> ApiResult<()>;

    async fn put_model(&self, model: ModelRow) -> ApiResult<()>;
    async fn get_model(
        &self,
        version: &common_types::ids::ModelVersion,
    ) -> Result<Option<ModelRow>, ApiError>;
    async fn set_model_status(
        &self,
        version: &common_types::ids::ModelVersion,
        status: ModelStatus,
    ) -> ApiResult<()>;

    /// All snapshots in `ts` order filtered by the basic dashboard query.
    async fn query_snapshots(
        &self,
        zone_id: Option<&ZoneId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        only_flagged: bool,
    ) -> Result<Vec<SnapshotRow>, ApiError>;
}

struct MemStore {
    zones: HashMap<ZoneId, Zone>,
    zone_order: Vec<ZoneId>,
    calibrations: Vec<CalibrationSnapshotRow>,
    events: Vec<PackageCountEvent>,
    corrections: HashMap<Uuid, AuditCorrectionRow>,
    rl_rows: Vec<RlTransitionRow>,
    snapshots: HashMap<Uuid, SnapshotRow>,
    models: HashMap<common_types::ids::ModelVersion, ModelRow>,
}

impl MemStore {
    fn new() -> Self {
        Self {
            zones: HashMap::new(),
            zone_order: Vec::new(),
            calibrations: Vec::new(),
            events: Vec::new(),
            corrections: HashMap::new(),
            rl_rows: Vec::new(),
            snapshots: HashMap::new(),
            models: HashMap::new(),
        }
    }
}

fn same_day(ts: DateTime<Utc>, date_ymd: (i32, u32, u32)) -> bool {
    ts.year() == date_ymd.0 && ts.month() == date_ymd.1 && ts.day() == date_ymd.2
}

pub struct InMemoryRepo {
    store: Mutex<MemStore>,
}

impl InMemoryRepo {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(MemStore::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, MemStore> {
        // A panic in another thread must not wedge the whole server;
        // recovered state in the in-memory store is best-effort anyway.
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for InMemoryRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Repos for InMemoryRepo {
    async fn put_zone(&self, zone: Zone) -> Result<(), ApiError> {
        let mut store = self.lock();
        if !store.zone_order.contains(&zone.zone_id) {
            store.zone_order.push(zone.zone_id.clone());
        }
        store.zones.insert(zone.zone_id.clone(), zone);
        Ok(())
    }

    async fn get_zone(&self, zone_id: &ZoneId) -> Result<Option<Zone>, ApiError> {
        Ok(self.lock().zones.get(zone_id).cloned())
    }

    async fn list_zones(&self) -> Result<Vec<Zone>, ApiError> {
        let store = self.lock();
        Ok(store
            .zone_order
            .iter()
            .filter_map(|z| store.zones.get(z).cloned())
            .collect())
    }

    async fn retire_zone(&self, zone_id: &ZoneId) -> ApiResult<()> {
        let mut store = self.lock();
        store.zones.remove(zone_id);
        store.zone_order.retain(|z| z != zone_id);
        Ok(())
    }

    async fn put_calibration(&self, snapshot: CalibrationSnapshotRow) -> ApiResult<()> {
        self.lock().calibrations.push(snapshot);
        Ok(())
    }

    async fn latest_calibration(&self) -> Result<Option<CalibrationSnapshotRow>, ApiError> {
        let store = self.lock();
        Ok(store.calibrations.iter().max_by_key(|c| c.ts).cloned())
    }

    async fn insert_count_event(&self, event: PackageCountEvent) -> ApiResult<()> {
        self.lock().events.push(event);
        Ok(())
    }

    async fn count_events(
        &self,
        zone_id: &ZoneId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<PackageCountEvent>, ApiError> {
        if to < from {
            return Err(ApiError::BadRequest("to < from".to_string()));
        }
        let store = self.lock();
        let mut events = store
            .events
            .iter()
            .filter(|e| &e.zone_id == zone_id && e.ts_from >= from && e.ts_from <= to)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|e| e.ts_from);
        Ok(events)
    }

    async fn put_correction(&self, row: AuditCorrectionRow) -> ApiResult<()> {
        self.lock().corrections.insert(row.correction_id, row);
        Ok(())
    }

    async fn get_correction(&self, id: Uuid) -> Result<Option<AuditCorrectionRow>, ApiError> {
        Ok(self.lock().corrections.get(&id).cloned())
    }

    async fn set_correction_outcome(&self, id: Uuid, outcome: CorrectionOutcome) -> ApiResult<()> {
        let mut store = self.lock();
        let row = store
            .corrections
            .get_mut(&id)
            .ok_or_else(|| ApiError::NotFound(format!("correction {id}")))?;
        row.outcome = outcome;
        Ok(())
    }

    async fn pending_corrections(&self) -> Result<Vec<AuditCorrectionRow>, ApiError> {
        let store = self.lock();
        Ok(store
            .corrections
            .values()
            .filter(|c| c.outcome == CorrectionOutcome::Submitted)
            .cloned()
            .collect())
    }

    async fn put_rl_transition(&self, row: RlTransitionRow) -> ApiResult<()> {
        self.lock().rl_rows.push(row);
        Ok(())
    }

    async fn rl_history(
        &self,
        date_ymd: (i32, u32, u32),
    ) -> Result<Vec<RlTransitionRow>, ApiError> {
        let store = self.lock();
        let mut rows = store
            .rl_rows
            .iter()
            .filter(|r| same_day(r.ts, date_ymd))
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|r| std::cmp::Reverse(r.ts));
        Ok(rows)
    }

    async fn put_snapshot(&self, row: SnapshotRow) -> ApiResult<()> {
        self.lock().snapshots.insert(row.frame_id, row);
        Ok(())
    }

    async fn get_snapshot(&self, frame_id: Uuid) -> Result<Option<SnapshotRow>, ApiError> {
        Ok(self.lock().snapshots.get(&frame_id).cloned())
    }

    async fn set_snapshot_promotion(
        &self,
        frame_id: Uuid,
        promotion: SnapshotPromotion,
    ) -> ApiResult<()> {
        let mut store = self.lock();
        let row = store
            .snapshots
            .get_mut(&frame_id)
            .ok_or_else(|| ApiError::NotFound(format!("snapshot {frame_id}")))?;
        row.promotion = promotion;
        Ok(())
    }

    async fn review_snapshot(
        &self,
        frame_id: Uuid,
        reviewed_by: OperatorId,
        coco_annotations: Option<Value>,
    ) -> ApiResult<()> {
        let mut store = self.lock();
        let row = store
            .snapshots
            .get_mut(&frame_id)
            .ok_or_else(|| ApiError::NotFound(format!("snapshot {frame_id}")))?;
        row.reviewed_by = Some(reviewed_by);
        row.reviewed_at = Some(Utc::now());
        if let Some(coco) = coco_annotations {
            row.coco_annotations = Some(coco);
        }
        Ok(())
    }

    async fn put_model(&self, model: ModelRow) -> ApiResult<()> {
        self.lock().models.insert(model.version.clone(), model);
        Ok(())
    }

    async fn get_model(
        &self,
        version: &common_types::ids::ModelVersion,
    ) -> Result<Option<ModelRow>, ApiError> {
        Ok(self.lock().models.get(version).cloned())
    }

    async fn set_model_status(
        &self,
        version: &common_types::ids::ModelVersion,
        status: ModelStatus,
    ) -> ApiResult<()> {
        let mut store = self.lock();
        let row = store
            .models
            .get_mut(version)
            .ok_or_else(|| ApiError::NotFound(format!("model {version}")))?;
        row.status = status;
        if status == ModelStatus::Active {
            row.activated_at = Some(Utc::now());
        }
        Ok(())
    }

    async fn query_snapshots(
        &self,
        zone_id: Option<&ZoneId>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        only_flagged: bool,
    ) -> Result<Vec<SnapshotRow>, ApiError> {
        let store = self.lock();
        let mut rows = store
            .snapshots
            .values()
            .filter(|s| {
                zone_id.is_none_or(|z| z == &s.zone_id)
                    && from.is_none_or(|f| s.ts >= f)
                    && to.is_none_or(|t| s.ts <= t)
                    && (!only_flagged || s.uncertainty_mask != 0)
            })
            .cloned()
            .collect::<Vec<_>>();
        // Hard row cap matching the SQL LIMIT 500 in db.rs.
        rows.truncate(500);
        rows.sort_by_key(|s| s.ts);
        Ok(rows)
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_types::action::RlAction;
    use common_types::bounds::DetectorThresholds;
    use common_types::counting::UncertaintyFlags;
    use common_types::fusion::ModalityBlend;
    use common_types::ids::{ModelVersion, TrackId};

    fn ts(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 7, h, 0, 0).unwrap()
    }

    fn event(h: u32, delta: i64) -> PackageCountEvent {
        PackageCountEvent::try_new(
            ts(h),
            ts(h),
            ZoneId::try_new("line_1").expect("z"),
            delta,
            delta,
            0.9,
            UncertaintyFlags::empty(),
            ModalityBlend::normalize(0.7),
            DetectorThresholds::try_new(0.45, 0.45, 45).expect("t"),
            vec![TrackId::new(1)],
            vec![],
            ModelVersion::try_new("m").expect("m"),
        )
        .expect("e")
    }

    #[tokio::test]
    async fn roundtrips_each_table() {
        let repo = InMemoryRepo::new();
        // Zones
        let zone =
            Zone::try_new(ZoneId::try_new("line_1").expect("z"), "Line 1".into()).expect("zone");
        repo.put_zone(zone.clone()).await.expect("put");
        assert_eq!(
            repo.get_zone(&zone.zone_id).await.expect("get"),
            Some(zone.clone())
        );
        assert_eq!(repo.list_zones().await.expect("list").len(), 1);
        repo.retire_zone(&zone.zone_id).await.expect("retire");
        assert_eq!(repo.list_zones().await.expect("list").len(), 0);

        // Events
        repo.insert_count_event(event(1, 10)).await.expect("ins");
        let from = Utc.with_ymd_and_hms(2026, 4, 7, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 4, 7, 23, 0, 0).unwrap();
        let zone = ZoneId::try_new("line_1").expect("z");
        let fetched = repo.count_events(&zone, from, to).await.expect("fetch");
        assert_eq!(fetched.len(), 1);

        // Corrections
        let feedback = HumanAuditFeedback::try_new(
            ZoneId::try_new("line_1").expect("z"),
            10,
            12,
            ts(1),
            ts(2),
            common_types::feedback::FailureTag::MISSED_COUNT,
            Some("under-counted".into()),
            common_types::ids::OperatorId::try_new("op").expect("op"),
            true,
        )
        .expect("fb");
        let row = AuditCorrectionRow {
            correction_id: Uuid::now_v7(),
            feedback,
            outcome: CorrectionOutcome::Submitted,
            reward_computed: Some(-0.04),
            root_cause_report: None,
            created_at: Utc::now(),
        };
        repo.put_correction(row.clone()).await.expect("c");
        assert_eq!(repo.pending_corrections().await.expect("p").len(), 1);
        repo.set_correction_outcome(row.correction_id, CorrectionOutcome::Committed)
            .await
            .expect("up");
        assert_eq!(repo.pending_corrections().await.expect("p").len(), 0);
        assert_eq!(
            repo.get_correction(row.correction_id)
                .await
                .expect("g")
                .expect("row")
                .outcome,
            CorrectionOutcome::Committed
        );

        // RL
        repo.put_rl_transition(RlTransitionRow {
            transition_id: Uuid::now_v7(),
            ts: ts(3),
            zone_id: None,
            before: RlAction::defaults(),
            after: RlAction::defaults(),
            reward: None,
            was_clamped: false,
            rejected: false,
            reason: RlReason::EodCorrection,
        })
        .await
        .expect("rl");
        assert_eq!(
            repo.rl_history((2026, 4, 7)).await.expect("h").len(),
            1,
            "same UTC day"
        );
        assert_eq!(repo.rl_history((2026, 4, 8)).await.expect("h").len(), 0);

        // Snapshots
        let sid = Uuid::now_v7();
        repo.put_snapshot(SnapshotRow {
            frame_id: sid,
            zone_id: ZoneId::try_new("line_1").expect("z"),
            ts: ts(4),
            camera_id: common_types::ids::CameraId::try_new("cam_a").expect("c"),
            jpeg_lowres: Some(vec![1, 2, 3]),
            uncertainty_mask: 1,
            divergence: Some(0.3),
            state: serde_json::json!({}),
            promotion: SnapshotPromotion::Amber,
            vlm_root_cause: None,
            coco_annotations: None,
            reviewed_by: None,
            reviewed_at: None,
        })
        .await
        .expect("s");
        repo.set_snapshot_promotion(sid, SnapshotPromotion::AutoLabeled)
            .await
            .expect("p");
        assert_eq!(
            repo.get_snapshot(sid)
                .await
                .expect("g")
                .expect("row")
                .promotion,
            SnapshotPromotion::AutoLabeled
        );
        repo.review_snapshot(
            sid,
            common_types::ids::OperatorId::try_new("op").expect("o"),
            Some(serde_json::json!({"annotations": []})),
        )
        .await
        .expect("review");
        let got = repo.get_snapshot(sid).await.expect("g").expect("row");
        assert_eq!(got.reviewed_by.as_ref().expect("by").as_str(), "op");
        assert!(got.reviewed_at.is_some());
        assert!(got.coco_annotations.is_some());
        // Search rows must never leak jpeg in the serialized surface.
        let text = serde_json::to_string(&got).expect("serde");
        assert!(!text.contains("jpeg_lowres"), "skip(S) hides bytes: {text}");

        // Models
        let mv = ModelVersion::try_new("rfdetr-base-v3").expect("mv");
        repo.put_model(ModelRow {
            version: mv.clone(),
            kind: crate::repo::ModelKind::Detector,
            path: "/models/v3.onnx".into(),
            checksum: "ab".into(),
            size_bytes: Some(100),
            train_dataset_ref: None,
            eval_report: None,
            status: ModelStatus::Staged,
            activated_at: None,
        })
        .await
        .expect("m");
        repo.set_model_status(&mv, ModelStatus::Active)
            .await
            .expect("st");
        let m = repo.get_model(&mv).await.expect("g").expect("m");
        assert_eq!(m.status, ModelStatus::Active);
        assert!(m.activated_at.is_some());
        assert!(
            repo.set_model_status(&mv, ModelStatus::Retired)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn calibration_latest_is_timestamp_max() {
        let repo = InMemoryRepo::new();
        for (id, hour) in [
            (Uuid::now_v7(), 1),
            (Uuid::now_v7(), 3),
            (Uuid::now_v7(), 2),
        ] {
            repo.put_calibration(CalibrationSnapshotRow {
                id,
                ts: ts(hour),
                lifecycle: LifecycleState::Counting,
                reason: Some("morning".into()),
                vlm_diff: None,
                roi_changes: None,
                cameras_ok: Some(4),
                cameras_total: Some(4),
                csi_nodes_ok: Some(6),
                csi_nodes_total: Some(6),
            })
            .await
            .expect("c");
        }
        let latest = repo.latest_calibration().await.expect("l").expect("row");
        assert_eq!(latest.ts, ts(3), "max ts wins, not insertion order");
    }

    #[tokio::test]
    async fn count_events_rejects_reversed_window() {
        let repo = InMemoryRepo::new();
        let from = ts(2);
        let to = ts(1);
        let zone = ZoneId::try_new("line_1").expect("z");
        let res = repo.count_events(&zone, from, to).await;
        assert!(matches!(res, Err(ApiError::BadRequest(_))));
    }

    #[tokio::test]
    async fn unknown_row_updates_404() {
        let repo = InMemoryRepo::new();
        let id = Uuid::now_v7();
        let res = repo
            .set_correction_outcome(id, CorrectionOutcome::Committed)
            .await;
        assert!(matches!(res, Err(ApiError::NotFound(_))));
        let res = repo
            .set_snapshot_promotion(id, SnapshotPromotion::Trained)
            .await;
        assert!(matches!(res, Err(ApiError::NotFound(_))));
        let mv = ModelVersion::try_new("nope").expect("mv");
        let res = repo.set_model_status(&mv, ModelStatus::Active).await;
        assert!(matches!(res, Err(ApiError::NotFound(_))));
    }
}
