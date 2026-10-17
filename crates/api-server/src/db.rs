//! Transport selection (docs/10 §6): `DATABASE_URL` ⇒ sqlx Postgres pool;
//! otherwise the hermetic in-memory repo. The Postgres layer executes the
//! docs/11 migrations on connect and implements the same [`Repos`] trait.
//! Runtime (macro-free) queries keep compilation DB-free.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::ApiError;
use crate::repo::{
    AuditCorrectionRow, CalibrationSnapshotRow, CorrectionOutcome, InMemoryRepo, ModelKind,
    ModelRow, ModelStatus, Repos, RlReason, RlTransitionRow, SnapshotPromotion, SnapshotRow, Zone,
};

use common_types::bounds::DetectorThresholds;
use common_types::counting::{PackageCountEvent, UncertaintyFlags};
use common_types::feedback::HumanAuditFeedback;
use common_types::fusion::ModalityBlend;
use common_types::ids::{CameraId, ModelVersion, OperatorId, TrackId, ZoneId};
use common_types::lifecycle::LifecycleState;

pub async fn build_repos(database_url: &Option<String>) -> Arc<dyn Repos> {
    match database_url {
        Some(url) => match PgRepo::connect(url).await {
            Ok(pg) => {
                tracing::info!(database_url = "ok", "pg pool connected, migrations applied");
                Arc::new(pg)
            }
            Err(e) => {
                tracing::error!("pg connect failed ({e}); falling back to in-memory repo");
                Arc::new(InMemoryRepo::new())
            }
        },
        None => Arc::new(InMemoryRepo::new()),
    }
}

pub struct PgRepo {
    pool: PgPool,
}

fn oops<E: std::fmt::Display>(e: E) -> ApiError {
    ApiError::Internal(e.to_string())
}

impl PgRepo {
    pub async fn connect(url: &str) -> Result<Self, String> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(|e| e.to_string())?;
        run_migrations(&pool).await?;
        Ok(Self { pool })
    }
}

const MIGRATION_0001: &str = include_str!("../migrations/0001_shado_core.sql");

/// Docs/11 migrations, wrapped in a transaction (§conventions). The
/// TimescaleDB statements inside are extension-guarded so a plain pg
/// degrades instead of failing boot. `raw_sql` uses the simple protocol
/// so the full multi-statement batch parses in one go.
async fn run_migrations(pool: &sqlx::PgPool) -> Result<(), String> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("migration tx begin: {e}"))?;
    sqlx::raw_sql(MIGRATION_0001)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("migration 0001 failed: {e}"))?;
    tx.commit()
        .await
        .map_err(|e| format!("migration tx commit: {e}"))?;
    tracing::info!("migration 0001_shado_core applied");
    Ok(())
}

fn parse_state(raw: &str) -> Option<LifecycleState> {
    serde_json::from_str::<LifecycleState>(&format!("\"{raw}\"")).ok()
}

fn day_bounds(date_ymd: (i32, u32, u32)) -> (DateTime<Utc>, DateTime<Utc>) {
    let day = Utc
        .with_ymd_and_hms(date_ymd.0, date_ymd.1, date_ymd.2, 0, 0, 0)
        .earliest()
        .unwrap_or_else(Utc::now);
    (day, day + Duration::hours(24))
}

fn to_zone(row: &sqlx::postgres::PgRow) -> Result<Zone, String> {
    let id: String = row
        .try_get("zone_id")
        .map_err(|e| format!("zone_id: {e}"))?;
    let camera_ids: Vec<String> = row
        .try_get("camera_ids")
        .map_err(|e| format!("camera_ids: {e}"))?;
    let csi: Vec<i16> = row
        .try_get("csi_node_ids")
        .map_err(|e| format!("csi_node_ids: {e}"))?;
    let roi: serde_json::Value = row
        .try_get("roi_json")
        .map_err(|e| format!("roi_json: {e}"))?;
    Ok(Zone {
        zone_id: ZoneId::try_new(id).map_err(|e| format!("{e:?}"))?,
        display_name: row
            .try_get("display_name")
            .map_err(|e| format!("display_name: {e}"))?,
        camera_ids: camera_ids
            .into_iter()
            .map(|c| CameraId::try_new(c).map_err(|e| format!("{e:?}")))
            .collect::<Result<Vec<_>, _>>()?,
        csi_node_ids: csi.into_iter().map(|n| n as u16).collect::<Vec<u16>>(),
        roi_lines: serde_json::from_value(roi).map_err(|e| format!("roi parse: {e}"))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| format!("created_at: {e}"))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| format!("updated_at: {e}"))?,
    })
}

#[async_trait::async_trait]
impl Repos for PgRepo {
    // ---------- zones ----------

    async fn put_zone(&self, zone: Zone) -> Result<(), ApiError> {
        let cameras: Vec<String> = zone.camera_ids.iter().map(|c| c.to_string()).collect();
        let csi: Vec<i16> = zone.csi_node_ids.iter().map(|n| *n as i16).collect();
        let roi =
            serde_json::to_value(&zone.roi_lines).map_err(|e| ApiError::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO zones (zone_id, display_name, camera_ids, csi_node_ids, roi_json)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (zone_id) DO UPDATE
             SET display_name = EXCLUDED.display_name, camera_ids = EXCLUDED.camera_ids,
                 csi_node_ids = EXCLUDED.csi_node_ids, roi_json = EXCLUDED.roi_json,
                 updated_at = now()",
        )
        .bind(zone.zone_id.as_str())
        .bind(&zone.display_name)
        .bind(&cameras)
        .bind(&csi)
        .bind(&roi)
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn get_zone(&self, zone_id: &ZoneId) -> Result<Option<Zone>, ApiError> {
        let row = sqlx::query(
            "SELECT zone_id, display_name, camera_ids, csi_node_ids, roi_json, created_at, updated_at
             FROM zones WHERE zone_id = $1 AND retired_at IS NULL",
        )
        .bind(zone_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(oops)?;
        row.map(|r| to_zone(&r).map_err(ApiError::Internal))
            .transpose()
    }

    async fn list_zones(&self) -> Result<Vec<Zone>, ApiError> {
        let rows = sqlx::query(
            "SELECT zone_id, display_name, camera_ids, csi_node_ids, roi_json, created_at, updated_at
             FROM zones WHERE retired_at IS NULL ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?;
        rows.iter()
            .map(to_zone)
            .collect::<Result<Vec<_>, String>>()
            .map_err(ApiError::Internal)
    }

    /// Soft delete: `zones` rows are never dropped (docs/11 §conventions).
    async fn retire_zone(&self, zone_id: &ZoneId) -> Result<(), ApiError> {
        let res = sqlx::query(
            "UPDATE zones SET retired_at = now(), updated_at = now() WHERE zone_id = $1",
        )
        .bind(zone_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        if res.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("zone {zone_id}")));
        }
        Ok(())
    }

    // ---------- calibration ----------

    async fn put_calibration(&self, snapshot: CalibrationSnapshotRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO calibration_state
             (id, ts, lifecycle, reason, vlm_diff, roi_changes, cameras_ok, cameras_total, csi_nodes_ok, csi_nodes_total)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(snapshot.id)
        .bind(snapshot.ts)
        .bind(snapshot.lifecycle.as_str())
        .bind(&snapshot.reason)
        .bind(&snapshot.vlm_diff)
        .bind(&snapshot.roi_changes)
        .bind(snapshot.cameras_ok.map(|v| v as i16))
        .bind(snapshot.cameras_total.map(|v| v as i16))
        .bind(snapshot.csi_nodes_ok.map(|v| v as i16))
        .bind(snapshot.csi_nodes_total.map(|v| v as i16))
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn latest_calibration(&self) -> Result<Option<CalibrationSnapshotRow>, ApiError> {
        let row = sqlx::query::<sqlx::Postgres>(
            "SELECT id, ts, lifecycle, reason, vlm_diff, roi_changes,
                    cameras_ok, cameras_total, csi_nodes_ok, csi_nodes_total
             FROM calibration_state ORDER BY ts DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(oops)?;
        let row = match row {
            None => return Ok(None),
            Some(row) => row,
        };
        let lifecycle_raw: String = row.try_get("lifecycle").map_err(oops)?;
        Ok(Some(CalibrationSnapshotRow {
            id: row.try_get("id").map_err(oops)?,
            ts: row.try_get("ts").map_err(oops)?,
            lifecycle: parse_state(&lifecycle_raw)
                .ok_or_else(|| ApiError::Internal(format!("bad lifecycle {lifecycle_raw}")))?,
            reason: row.try_get("reason").map_err(oops)?,
            vlm_diff: row.try_get("vlm_diff").map_err(oops)?,
            roi_changes: row.try_get("roi_changes").map_err(oops)?,
            cameras_ok: row
                .try_get::<Option<i16>, _>("cameras_ok")
                .map_err(oops)?
                .map(|v| v as u16),
            cameras_total: row
                .try_get::<Option<i16>, _>("cameras_total")
                .map_err(oops)?
                .map(|v| v as u16),
            csi_nodes_ok: row
                .try_get::<Option<i16>, _>("csi_nodes_ok")
                .map_err(oops)?
                .map(|v| v as u16),
            csi_nodes_total: row
                .try_get::<Option<i16>, _>("csi_nodes_total")
                .map_err(oops)?
                .map(|v| v as u16),
        }))
    }

    // ---------- count_events ----------

    async fn insert_count_event(&self, event: PackageCountEvent) -> Result<(), ApiError> {
        let track_ids: Vec<i64> = event.track_ids.iter().map(|t| t.as_u64() as i64).collect();
        sqlx::query(
            "INSERT INTO count_events
             (event_id, zone_id, ts_from, ts_to, count_delta, cumulative_after, confidence,
              uncertainty_mask, modality_vision, modality_wifi, conf_threshold, iou_threshold,
              track_buffer, track_ids, model_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(event.event_id)
        .bind(event.zone_id.as_str())
        .bind(event.ts_from)
        .bind(event.ts_to)
        .bind(i32::try_from(event.count_delta).map_err(|e| ApiError::Internal(e.to_string()))?)
        .bind(event.cumulative.cumulative_after)
        .bind(event.confidence)
        .bind(event.uncertainty.bits() as i16)
        .bind(event.modality_blend.vision())
        .bind(event.modality_blend.wifi())
        .bind(event.thresholds_used.conf_threshold)
        .bind(event.thresholds_used.iou_threshold)
        .bind(
            i16::try_from(event.thresholds_used.track_buffer_frames)
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        )
        .bind(&track_ids)
        .bind(event.model_version.as_str())
        .execute(&self.pool)
        .await
        .map_err(oops)?;
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
        let rows = sqlx::query(
            "SELECT event_id, ts_from, ts_to, count_delta, cumulative_after, confidence,
                    uncertainty_mask, modality_vision, modality_wifi, conf_threshold,
                    iou_threshold, track_buffer, track_ids, model_version
             FROM count_events
             WHERE zone_id = $1 AND ts_from >= $2 AND ts_from <= $3
             ORDER BY ts_from",
        )
        .bind(zone_id.as_str())
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?;
        rows.into_iter()
            .map(|row| -> Result<PackageCountEvent, ApiError> {
                let zone = zone_id.clone();
                let ts_from: DateTime<Utc> = row.try_get("ts_from").map_err(oops)?;
                let ts_to: DateTime<Utc> = row.try_get("ts_to").map_err(oops)?;
                let count_delta: i32 = row.try_get("count_delta").map_err(oops)?;
                let cumulative_after: i64 = row.try_get("cumulative_after").map_err(oops)?;
                let confidence: f32 = row.try_get("confidence").map_err(oops)?;
                let mask: i16 = row.try_get("uncertainty_mask").map_err(oops)?;
                let vision: f32 = row.try_get("modality_vision").map_err(oops)?;
                let wifi: f32 = row.try_get("modality_wifi").map_err(oops)?;
                let conf: Option<f32> = row.try_get("conf_threshold").map_err(oops)?;
                let iou: Option<f32> = row.try_get("iou_threshold").map_err(oops)?;
                let buffer: Option<i16> = row.try_get("track_buffer").map_err(oops)?;
                let track_ids: Vec<i64> = row.try_get("track_ids").map_err(oops)?;
                let mv_raw: String = row.try_get("model_version").map_err(oops)?;
                let th = DetectorThresholds::try_new(
                    conf.unwrap_or(0.45),
                    iou.unwrap_or(0.45),
                    buffer.unwrap_or(45) as u16,
                )
                .map_err(|e| ApiError::Internal(format!("{e:?}")))?;
                let tracks: Vec<TrackId> =
                    track_ids.iter().map(|t| TrackId::new(*t as u64)).collect();
                PackageCountEvent::try_new(
                    ts_from,
                    ts_to,
                    zone,
                    i64::from(count_delta),
                    cumulative_after,
                    confidence,
                    UncertaintyFlags::from_bits_truncate(mask.clamp(0, i16::MAX) as u16),
                    ModalityBlend::new(vision, wifi)
                        .unwrap_or_else(|_| ModalityBlend::normalize(vision)),
                    th,
                    tracks,
                    vec![],
                    ModelVersion::try_new(&mv_raw)
                        .map_err(|e| ApiError::Internal(format!("{e:?}")))?,
                )
                .map_err(|e| ApiError::Internal(format!("{e:?}")))
            })
            .collect()
    }

    // ---------- audit corrections ----------

    async fn put_correction(&self, row: AuditCorrectionRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO audit_corrections
             (correction_id, zone_id, reported_count, actual_count, ts_from, ts_to,
              failure_tags, notes, operator_id, outcome, reward_computed, retrain_requested, root_cause_report)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (correction_id) DO UPDATE SET outcome = EXCLUDED.outcome",
        )
        .bind(row.correction_id)
        .bind(row.feedback.zone_id.as_str())
        .bind(row.feedback.reported_count)
        .bind(row.feedback.actual_count)
        .bind(row.feedback.ts_from)
        .bind(row.feedback.ts_to)
        .bind(row.feedback.failure_tags.to_db_mask())
        .bind(&row.feedback.notes)
        .bind(row.feedback.operator_id.as_str())
        .bind(row.outcome.as_db())
        .bind(row.reward_computed)
        .bind(row.feedback.wants_retrain)
        .bind(&row.root_cause_report)
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn get_correction(&self, id: Uuid) -> Result<Option<AuditCorrectionRow>, ApiError> {
        let row = sqlx::query(
            "SELECT correction_id, zone_id, reported_count, actual_count, ts_from, ts_to,
                    failure_tags, notes, operator_id, outcome, reward_computed,
                    retrain_requested, root_cause_report, created_at
             FROM audit_corrections WHERE correction_id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(oops)?;
        let row = match row {
            None => return Ok(None),
            Some(row) => row,
        };
        let zone_raw: String = row.try_get("zone_id").map_err(oops)?;
        let op_raw: String = row.try_get("operator_id").map_err(oops)?;
        let tags: i16 = row.try_get("failure_tags").map_err(oops)?;
        let notes: Option<String> = row.try_get("notes").map_err(oops)?;
        let reported: i64 = row.try_get("reported_count").map_err(oops)?;
        let actual: i64 = row.try_get("actual_count").map_err(oops)?;
        let ts_from: DateTime<Utc> = row.try_get("ts_from").map_err(oops)?;
        let ts_to: DateTime<Utc> = row.try_get("ts_to").map_err(oops)?;
        let wants: bool = row.try_get("retrain_requested").map_err(oops)?;
        let feedback = HumanAuditFeedback::try_new(
            ZoneId::try_new(&zone_raw).map_err(|e| ApiError::Internal(format!("{e:?}")))?,
            reported,
            actual,
            ts_from,
            ts_to,
            common_types::feedback::FailureTag::from_db_mask(tags),
            notes,
            OperatorId::try_new(&op_raw).map_err(|e| ApiError::Internal(format!("{e:?}")))?,
            wants,
        )
        .map_err(|e| ApiError::Internal(format!("{e:?}")))?;
        let outcome_raw: String = row.try_get("outcome").map_err(oops)?;
        Ok(Some(AuditCorrectionRow {
            correction_id: row.try_get("correction_id").map_err(oops)?,
            feedback,
            outcome: CorrectionOutcome::parse_db(&outcome_raw)
                .ok_or_else(|| ApiError::Internal(format!("bad outcome {outcome_raw}")))?,
            reward_computed: row.try_get("reward_computed").map_err(oops)?,
            root_cause_report: row.try_get("root_cause_report").map_err(oops)?,
            created_at: row.try_get("created_at").map_err(oops)?,
        }))
    }

    async fn set_correction_outcome(
        &self,
        id: Uuid,
        outcome: CorrectionOutcome,
    ) -> Result<(), ApiError> {
        let res = sqlx::query("UPDATE audit_corrections SET outcome = $2 WHERE correction_id = $1")
            .bind(id)
            .bind(outcome.as_db())
            .execute(&self.pool)
            .await
            .map_err(oops)?;
        if res.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("correction {id}")));
        }
        Ok(())
    }

    async fn pending_corrections(&self) -> Result<Vec<AuditCorrectionRow>, ApiError> {
        let ids: Vec<Uuid> = sqlx::query(
            "SELECT correction_id, ts_from FROM audit_corrections
             WHERE outcome = 'submitted' ORDER BY ts_from",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?
        .into_iter()
        .filter_map(|r| r.try_get::<Uuid, _>("correction_id").ok())
        .collect();
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(row) = self.get_correction(id).await? {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    // ---------- rl_audit ----------

    async fn put_rl_transition(&self, row: RlTransitionRow) -> Result<(), ApiError> {
        let before =
            serde_json::to_value(row.before).map_err(|e| ApiError::Internal(e.to_string()))?;
        let after =
            serde_json::to_value(row.after).map_err(|e| ApiError::Internal(e.to_string()))?;
        sqlx::query(
            "INSERT INTO rl_audit (transition_id, ts, zone_id, \"before\", \"after\", reward,
                    was_clamped, rejected, reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(row.transition_id)
        .bind(row.ts)
        .bind(row.zone_id.as_ref().map(|z| z.as_str()))
        .bind(&before)
        .bind(&after)
        .bind(row.reward)
        .bind(row.was_clamped)
        .bind(row.rejected)
        .bind(row.reason.as_db())
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn rl_history(
        &self,
        date_ymd: (i32, u32, u32),
    ) -> Result<Vec<RlTransitionRow>, ApiError> {
        let (start, end) = day_bounds(date_ymd);
        let rows = sqlx::query(
            "SELECT transition_id, ts, zone_id, \"before\", \"after\", reward,
                    was_clamped, rejected, reason
             FROM rl_audit WHERE ts >= $1 AND ts < $2 ORDER BY ts DESC",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?;
        rows.into_iter()
            .map(|row| -> Result<RlTransitionRow, ApiError> {
                let reason_raw: String = row.try_get("reason").map_err(oops)?;
                let before: serde_json::Value = row.try_get("before").map_err(oops)?;
                let after: serde_json::Value = row.try_get("after").map_err(oops)?;
                let zone_raw: Option<String> = row.try_get("zone_id").map_err(oops)?;
                Ok(RlTransitionRow {
                    transition_id: row.try_get("transition_id").map_err(oops)?,
                    ts: row.try_get("ts").map_err(oops)?,
                    zone_id: zone_raw
                        .map(|z| {
                            ZoneId::try_new(&z).map_err(|e| ApiError::Internal(format!("{e:?}")))
                        })
                        .transpose()?,
                    before: serde_json::from_value(before)
                        .map_err(|e| ApiError::Internal(e.to_string()))?,
                    after: serde_json::from_value(after)
                        .map_err(|e| ApiError::Internal(e.to_string()))?,
                    reward: row.try_get("reward").map_err(oops)?,
                    was_clamped: row.try_get("was_clamped").map_err(oops)?,
                    rejected: row.try_get("rejected").map_err(oops)?,
                    reason: RlReason::parse_db(&reason_raw)
                        .ok_or_else(|| ApiError::Internal(format!("bad reason {reason_raw}")))?,
                })
            })
            .collect()
    }

    // ---------- snapshots ----------

    async fn put_snapshot(&self, row: SnapshotRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO uncertain_snapshots
             (frame_id, zone_id, ts, camera_id, jpeg_lowres, uncertainty_mask, divergence,
              state, promotion, vlm_root_cause, coco_annotations, reviewed_by, reviewed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (frame_id) DO NOTHING",
        )
        .bind(row.frame_id)
        .bind(row.zone_id.as_str())
        .bind(row.ts)
        .bind(row.camera_id.as_str())
        .bind(&row.jpeg_lowres)
        .bind(i16::try_from(row.uncertainty_mask).map_err(|e| ApiError::Internal(e.to_string()))?)
        .bind(row.divergence)
        .bind(&row.state)
        .bind(row.promotion.as_db())
        .bind(&row.vlm_root_cause)
        .bind(&row.coco_annotations)
        .bind(row.reviewed_by.as_ref().map(|o| o.as_str()))
        .bind(row.reviewed_at)
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn get_snapshot(&self, frame_id: Uuid) -> Result<Option<SnapshotRow>, ApiError> {
        let rows = sqlx::query(
            "SELECT frame_id, zone_id, ts, camera_id, jpeg_lowres, uncertainty_mask,
                    divergence, state, promotion, vlm_root_cause, coco_annotations,
                    reviewed_by, reviewed_at
             FROM uncertain_snapshots WHERE frame_id = $1",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?;
        rows.first().map(to_snapshot).transpose()
    }

    async fn set_snapshot_promotion(
        &self,
        frame_id: Uuid,
        promotion: SnapshotPromotion,
    ) -> Result<(), ApiError> {
        let res = sqlx::query("UPDATE uncertain_snapshots SET promotion = $2 WHERE frame_id = $1")
            .bind(frame_id)
            .bind(promotion.as_db())
            .execute(&self.pool)
            .await
            .map_err(oops)?;
        if res.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("snapshot {frame_id}")));
        }
        Ok(())
    }

    async fn review_snapshot(
        &self,
        frame_id: Uuid,
        reviewed_by: OperatorId,
        coco_annotations: Option<serde_json::Value>,
    ) -> Result<(), ApiError> {
        let res = sqlx::query(
            "UPDATE uncertain_snapshots
             SET reviewed_by = $2, reviewed_at = now(),
                 coco_annotations = COALESCE($3, coco_annotations)
             WHERE frame_id = $1",
        )
        .bind(frame_id)
        .bind(reviewed_by.as_str())
        .bind(&coco_annotations)
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        if res.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("snapshot {frame_id}")));
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
        let rows = sqlx::query(
            "SELECT frame_id, zone_id, ts, camera_id, jpeg_lowres, uncertainty_mask,
                    divergence, state, promotion, vlm_root_cause, coco_annotations,
                    reviewed_by, reviewed_at
             FROM uncertain_snapshots
             WHERE ($1::text IS NULL OR zone_id = $1)
               AND ($2::timestamptz IS NULL OR ts >= $2)
               AND ($3::timestamptz IS NULL OR ts <= $3)
               AND ($4 = false OR uncertainty_mask <> 0)
             ORDER BY ts",
        )
        .bind(zone_id.map(|z| z.to_string()))
        .bind(from)
        .bind(to)
        .bind(only_flagged)
        .fetch_all(&self.pool)
        .await
        .map_err(oops)?;
        rows.iter()
            .map(to_snapshot)
            .collect::<Result<Vec<_>, ApiError>>()
    }

    // ---------- model registry ----------

    async fn put_model(&self, model: ModelRow) -> Result<(), ApiError> {
        let kind = match model.kind {
            ModelKind::Detector => "detector",
            ModelKind::Csi => "csi",
            ModelKind::Rl => "rl",
        };
        sqlx::query(
            "INSERT INTO model_registry (version, kind, path, checksum, size_bytes,
                    train_dataset_ref, eval_report, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (version) DO UPDATE SET kind = EXCLUDED.kind,
                 path = EXCLUDED.path, checksum = EXCLUDED.checksum,
                 size_bytes = EXCLUDED.size_bytes",
        )
        .bind(model.version.as_str())
        .bind(kind)
        .bind(&model.path)
        .bind(&model.checksum)
        .bind(model.size_bytes)
        .bind(&model.train_dataset_ref)
        .bind(&model.eval_report)
        .bind(model_status_raw(model.status))
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        Ok(())
    }

    async fn get_model(&self, version: &ModelVersion) -> Result<Option<ModelRow>, ApiError> {
        let row = sqlx::query(
            "SELECT version, kind, path, checksum, size_bytes, train_dataset_ref,
                    eval_report, status, activated_at
             FROM model_registry WHERE version = $1",
        )
        .bind(version.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(oops)?;
        let row = match row {
            None => return Ok(None),
            Some(row) => row,
        };
        let kind_raw: String = row.try_get("kind").map_err(oops)?;
        let status_raw: String = row.try_get("status").map_err(oops)?;
        Ok(Some(ModelRow {
            version: version.clone(),
            kind: match kind_raw.as_str() {
                "detector" => ModelKind::Detector,
                "csi" => ModelKind::Csi,
                "rl" => ModelKind::Rl,
                other => return Err(ApiError::Internal(format!("bad kind {other}"))),
            },
            path: row.try_get("path").map_err(oops)?,
            checksum: row.try_get("checksum").map_err(oops)?,
            size_bytes: row.try_get("size_bytes").map_err(oops)?,
            train_dataset_ref: row.try_get("train_dataset_ref").map_err(oops)?,
            eval_report: row.try_get("eval_report").map_err(oops)?,
            status: parse_model_status(&status_raw)
                .ok_or_else(|| ApiError::Internal(format!("bad status {status_raw}")))?,
            activated_at: row.try_get("activated_at").map_err(oops)?,
        }))
    }

    async fn set_model_status(
        &self,
        version: &ModelVersion,
        status: ModelStatus,
    ) -> Result<(), ApiError> {
        let raw = model_status_raw(status);
        let res = sqlx::query(
            "UPDATE model_registry SET status = $2,
                 activated_at = CASE WHEN $2 = 'active' THEN now() ELSE activated_at END
             WHERE version = $1",
        )
        .bind(version.as_str())
        .bind(raw)
        .execute(&self.pool)
        .await
        .map_err(oops)?;
        if res.rows_affected() == 0 {
            return Err(ApiError::NotFound(format!("model {version}")));
        }
        Ok(())
    }
}

fn model_status_raw(status: ModelStatus) -> &'static str {
    match status {
        ModelStatus::Staged => "staged",
        ModelStatus::Shadow => "shadow",
        ModelStatus::Active => "active",
        ModelStatus::Retired => "retired",
    }
}

fn parse_model_status(raw: &str) -> Option<ModelStatus> {
    Some(match raw {
        "staged" => ModelStatus::Staged,
        "shadow" => ModelStatus::Shadow,
        "active" => ModelStatus::Active,
        "retired" => ModelStatus::Retired,
        _ => return None,
    })
}

fn to_snapshot(row: &sqlx::postgres::PgRow) -> Result<SnapshotRow, ApiError> {
    let zone_raw: String = row.try_get("zone_id").map_err(oops)?;
    let cam_raw: String = row.try_get("camera_id").map_err(oops)?;
    let promotion_raw: String = row.try_get("promotion").map_err(oops)?;
    let mask: i16 = row.try_get("uncertainty_mask").map_err(oops)?;
    Ok(SnapshotRow {
        frame_id: row.try_get("frame_id").map_err(oops)?,
        zone_id: ZoneId::try_new(&zone_raw).map_err(|e| ApiError::Internal(format!("{e:?}")))?,
        ts: row.try_get("ts").map_err(oops)?,
        camera_id: CameraId::try_new(&cam_raw).map_err(|e| ApiError::Internal(format!("{e:?}")))?,
        jpeg_lowres: row.try_get("jpeg_lowres").map_err(oops)?,
        uncertainty_mask: mask.clamp(0, i16::MAX) as u16,
        divergence: row.try_get("divergence").map_err(oops)?,
        state: row.try_get("state").map_err(oops)?,
        promotion: SnapshotPromotion::parse_db(&promotion_raw)
            .ok_or_else(|| ApiError::Internal(format!("bad promotion {promotion_raw}")))?,
        vlm_root_cause: row.try_get("vlm_root_cause").map_err(oops)?,
        coco_annotations: row.try_get("coco_annotations").map_err(oops)?,
        reviewed_by: row
            .try_get::<Option<String>, _>("reviewed_by")
            .map_err(oops)?
            .map(|o| OperatorId::try_new(&o).map_err(|e| ApiError::Internal(format!("{e:?}"))))
            .transpose()?,
        reviewed_at: row.try_get("reviewed_at").map_err(oops)?,
    })
}

use chrono::TimeZone;

#[cfg(all(feature = "itest", test))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod itests {
    use super::*;

    fn db_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok()
    }

    use common_types::action::RlAction;

    async fn pg_repos() -> Option<Box<dyn Repos>> {
        let url = db_url()?;
        Some(Box::new(PgRepo::connect(&url).await.expect("pg connect")))
    }

    #[tokio::test]
    async fn pg_roundtrips_each_table() {
        use common_types::fusion::ModalityBlend;

        let Some(repos) = pg_repos().await else {
            tracing::warn!("DATABASE_URL missing: skipping pg integration test");
            return;
        };
        let zone_id = ZoneId::try_new("itest_line").expect("zone");
        Zone::try_new(zone_id.clone(), "Itest line".to_string()).expect("zone ctor");
        repos
            .put_zone(Zone::try_new(zone_id.clone(), "Itest line".to_string()).expect("z"))
            .await
            .expect("put_zone");
        let got = repos
            .get_zone(&zone_id)
            .await
            .expect("get_zone")
            .expect("row");
        assert_eq!(got.display_name, "Itest line");

        let ts = Utc.with_ymd_and_hms(2026, 4, 7, 12, 0, 0).unwrap();
        let event = PackageCountEvent::try_new(
            ts,
            ts,
            zone_id.clone(),
            5,
            5,
            0.94,
            UncertaintyFlags::OCCLUDED_ESTIMATE,
            ModalityBlend::normalize(0.7),
            DetectorThresholds::try_new(0.45, 0.45, 45).expect("th"),
            vec![TrackId::new(9)],
            vec![],
            ModelVersion::try_new("itest-m").expect("mv"),
        )
        .expect("event");
        repos.insert_count_event(event).await.expect("insert");
        let fetched = repos
            .count_events(&zone_id, ts, ts)
            .await
            .expect("count_events");
        assert_eq!(fetched.len(), 1, "same-timestamp range must hit");
        assert_eq!(fetched[0].count_delta, 5);
        assert!(fetched[0].is_occluded_estimate());
        assert_eq!(fetched[0].track_ids.len(), 1);

        let feedback = HumanAuditFeedback::try_new(
            zone_id.clone(),
            5,
            7,
            ts,
            ts,
            common_types::feedback::FailureTag::MISSED_COUNT,
            None,
            OperatorId::try_new("itest-op").expect("op"),
            false,
        )
        .expect("feedback");
        let correction = AuditCorrectionRow {
            correction_id: Uuid::now_v7(),
            feedback,
            outcome: CorrectionOutcome::Submitted,
            reward_computed: Some(-0.02),
            root_cause_report: None,
            created_at: Utc::now(),
        };
        repos
            .put_correction(correction.clone())
            .await
            .expect("put_corr");
        assert_eq!(repos.pending_corrections().await.expect("p").len(), 1);
        repos
            .set_correction_outcome(correction.correction_id, CorrectionOutcome::Committed)
            .await
            .expect("commit");
        assert_eq!(repos.pending_corrections().await.expect("p2").len(), 0);

        repos
            .put_calibration(CalibrationSnapshotRow {
                id: Uuid::now_v7(),
                ts,
                lifecycle: LifecycleState::Calibrating,
                reason: Some("itest".to_string()),
                vlm_diff: None,
                roi_changes: None,
                cameras_ok: Some(4),
                cameras_total: Some(4),
                csi_nodes_ok: Some(6),
                csi_nodes_total: Some(6),
            })
            .await
            .expect("cal");
        let latest = repos
            .latest_calibration()
            .await
            .expect("latest")
            .expect("row");
        assert_eq!(latest.lifecycle, LifecycleState::Calibrating);

        repos
            .put_rl_transition(RlTransitionRow {
                transition_id: Uuid::now_v7(),
                ts,
                zone_id: Some(zone_id),
                before: RlAction::defaults(),
                after: RlAction::defaults(),
                reward: None,
                was_clamped: true,
                rejected: false,
                reason: RlReason::EodCorrection,
            })
            .await
            .expect("rl");
        let hist = repos.rl_history((2026, 4, 7)).await.expect("rl history");
        assert!(hist.iter().any(|r| r.was_clamped));

        let frame = Uuid::now_v7();
        repos
            .put_snapshot(SnapshotRow {
                frame_id: frame,
                zone_id: ZoneId::try_new("itest_line").expect("z"),
                ts,
                camera_id: CameraId::try_new("camA").expect("c"),
                jpeg_lowres: None,
                uncertainty_mask: 1,
                divergence: Some(0.25),
                state: serde_json::json!({}),
                promotion: SnapshotPromotion::Amber,
                vlm_root_cause: None,
                coco_annotations: None,
                reviewed_by: None,
                reviewed_at: None,
            })
            .await
            .expect("snap");
        let flagged = repos
            .query_snapshots(
                Some(&ZoneId::try_new("itest_line").expect("z")),
                None,
                None,
                true,
            )
            .await
            .expect("q");
        assert!(flagged.iter().any(|s| s.frame_id == frame));
        let mv = ModelVersion::try_new("itest-reg").expect("mv");
        repos
            .put_model(ModelRow {
                version: mv.clone(),
                kind: ModelKind::Detector,
                path: "/tmp/v".into(),
                checksum: "00".into(),
                size_bytes: Some(12),
                train_dataset_ref: None,
                eval_report: None,
                status: ModelStatus::Staged,
                activated_at: None,
            })
            .await
            .expect("model");
        repos
            .set_model_status(&mv, ModelStatus::Active)
            .await
            .expect("st");
        let m = repos.get_model(&mv).await.expect("g").expect("m");
        assert_eq!(m.status, ModelStatus::Active);
        assert!(m.activated_at.is_some());
    }
}
