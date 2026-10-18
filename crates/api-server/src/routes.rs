//! The axum surface (docs/10 §1 route map) plus the /ws/live pump and the
//! defense layers (timeout, body caps, CORS, trace-id + metrics middleware).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use common_types::action::RlAction;
use common_types::feedback::{FailureTag, HumanAuditFeedback};
use common_types::ids::{OperatorId, ZoneId};
use common_types::lifecycle::LifecycleState;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use uuid::Uuid;

use crate::agg::{aggregate, hourly_curve};
use crate::error::ApiError;
use crate::repo::{
    AuditCorrectionRow, CalibrationSnapshotRow, CorrectionOutcome, RlReason, RlTransitionRow,
    RoiLine, SnapshotPromotion,
};
use crate::state::{AppState, RequestMetrics, route_tag};
use crate::ws::{WsEvent, parse_topics, to_wire};

pub type Shared = Arc<AppState>;

/// A correction that moved the machine count more than this far from the
/// human truth requires the double confirmation on commit (docs/10 §1).
const BIG_DELTA_CONFIRM: i64 = 100;

fn parse_lifecycle(raw: &str) -> Option<LifecycleState> {
    Some(match raw {
        "Standby" => LifecycleState::Standby,
        "Calibrating" => LifecycleState::Calibrating,
        "Counting" => LifecycleState::Counting,
        "Correcting" => LifecycleState::Correcting,
        "Degraded" => LifecycleState::Degraded,
        _ => return None,
    })
}

pub fn build_router(state: Shared) -> Router {
    let metrics: Arc<RequestMetrics> = Arc::new(state.metrics.clone());
    let with_counts = middleware::from_fn(move |req: Request<Body>, next: Next| {
        let metrics = metrics.clone();
        async move {
            metrics.bump(route_tag(req.uri().path()));
            let mut response = next.run(req).await;
            let trace = format!("{:032x}", uuid::Uuid::now_v7().as_u128());
            if let Ok(value) = HeaderValue::from_str(&trace) {
                response
                    .headers_mut()
                    .insert(HeaderName::from_static("x-shado-trace-id"), value);
            }
            response
        }
    });
    let origin = HeaderValue::from_str(&state.cfg.allowed_origin)
        .unwrap_or_else(|_| HeaderValue::from_static("http://localhost:3000"));
    let cors = CorsLayer::new().allow_origin(origin);
    let timeout = Duration::from_millis(state.cfg.request_timeout_ms);
    let body_cap = state.cfg.body_cap_bytes;

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_route))
        .route("/api/v1/lifecycle", get(get_lifecycle))
        .route("/api/v1/lifecycle/force", post(force_lifecycle))
        .route("/api/v1/calibration/latest", get(get_calibration))
        .route(
            "/api/v1/calibration/rois/{zone_id}",
            post(put_rois).layer(axum::extract::DefaultBodyLimit::max(
                state.cfg.roi_body_cap_bytes,
            )),
        )
        .route("/api/v1/zones/{zone_id}/counts", get(get_counts))
        .route("/api/v1/zones/{zone_id}/hourly", get(get_hourly))
        .route("/api/v1/audit/pending", get(get_pending))
        .route("/api/v1/audit/corrections", post(post_correction))
        .route(
            "/api/v1/audit/corrections/{id}/commit",
            post(commit_correction),
        )
        .route("/api/v1/rl/state", get(get_rl_state))
        .route("/api/v1/rl/override", post(post_rl_override))
        .route("/api/v1/rl/history", get(get_rl_history))
        .route("/api/v1/snapshots/query", post(query_snapshots))
        .route("/api/v1/labeling/queue", get(get_label_queue))
        .route("/api/v1/labeling/{frame_id}/approve", post(approve_label))
        .route("/ws/live", get(ws_live))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .with_state(state)
        .layer(with_counts)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            timeout,
        ))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(body_cap))
}

// ------------------- health / ops -------------------

/// Constant-time byte equality (folded XOR) so key checks do not leak
/// prefix length through early exits.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Guards mutating routes + operator exports while `SHADO_API_KEY` is
/// set; with no key configured the server keeps the docs/10 §6
/// trust-on-LAN posture.
async fn require_api_key(State(state): State<Shared>, req: Request<Body>, next: Next) -> Response {
    let Some(key) = state.cfg.api_key.clone() else {
        return next.run(req).await;
    };
    let provided = req
        .headers()
        .get("x-shado-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    match provided {
        Some(got) if ct_eq(got.as_bytes(), key.as_bytes()) => next.run(req).await,
        _ => ApiError::Unauthorized.into_response(),
    }
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

async fn readyz(State(state): State<Shared>) -> Response {
    let ok = state.repos.list_zones().await.is_ok();
    if ok {
        (StatusCode::OK, Json(json!({"ready": true}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ready": false, "repo": "error"})),
        )
            .into_response()
    }
}

async fn metrics_route(State(state): State<Shared>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("# TYPE shado_requests_total counter\n");
    for (route, n) in state.metrics.snapshot() {
        out.push_str(&format!("shado_requests_total{{route=\"{route}\"}} {n}\n"));
    }
    out.push_str("# TYPE shado_ws_clients gauge\n");
    out.push_str(&format!(
        "shado_ws_clients {}\n",
        state.hub.subscriber_count()
    ));
    out.push_str("# TYPE shado_uptime_seconds gauge\n");
    let uptime = (Utc::now() - state.started_at).num_seconds().max(0);
    out.push_str(&format!("shado_uptime_seconds {uptime}\n"));
    out
}

// ------------------- lifecycle + calibration -------------------

async fn get_lifecycle(State(state): State<Shared>) -> impl IntoResponse {
    let view = state.lifecycle().clone();
    Json(json!({
        "state": view.state.as_str(),
        "since": view.since,
        "reason": view.last_reason,
    }))
}

#[derive(Deserialize)]
struct ForceReq {
    to: String,
    reason: Option<String>,
}

async fn force_lifecycle(
    State(state): State<Shared>,
    Json(req): Json<ForceReq>,
) -> Result<Json<Value>, ApiError> {
    let to = parse_lifecycle(&req.to)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown lifecycle state {:?}", req.to)))?;
    let mut view = state.lifecycle();
    let from = view.state;
    if !LifecycleState::is_valid_transition(from, to) {
        return Err(ApiError::Conflict(format!(
            "{from} -> {to} violates docs/02 §7 transition matrix; ack handles any repair"
        )));
    }
    let now = Utc::now();
    view.state = to;
    view.since = now;
    view.last_reason = req.reason.clone();
    state
        .hub
        .push(WsEvent::LifecycleEvent { from, to, ts: now });
    Ok(Json(
        json!({"ok": true, "from": from.to_string(), "to": to.to_string()}),
    ))
}

async fn get_calibration(
    State(state): State<Shared>,
) -> Result<Json<CalibrationSnapshotRow>, ApiError> {
    let row = state
        .repos
        .latest_calibration()
        .await?
        .ok_or_else(|| ApiError::NotFound("no calibration snapshot yet".to_string()))?;
    Ok(Json(row))
}

#[derive(Deserialize)]
struct RoiSubmit {
    lines: Vec<Vec<(f32, f32)>>,
}

async fn put_rois(
    State(state): State<Shared>,
    Path(zone_id): Path<String>,
    Json(submit): Json<RoiSubmit>,
) -> Result<Json<Value>, ApiError> {
    let zone_id = ZoneId::try_new(&zone_id).map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    let z = state
        .repos
        .get_zone(&zone_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("zone {zone_id}")))?;
    if submit.lines.is_empty() {
        return Err(ApiError::BadRequest("lines must be non-empty".to_string()));
    }
    let mut zone = z;
    for pairs in &submit.lines {
        if pairs.len() < 2 {
            return Err(ApiError::BadRequest(
                "a counting line needs at least two points".to_string(),
            ));
        }
        zone.roi_lines.push(RoiLine {
            points: pairs.clone(),
        });
    }
    zone.updated_at = Utc::now();
    state.repos.put_zone(zone).await?;
    let applied = submit.lines.len();
    state
        .repos
        .put_calibration(CalibrationSnapshotRow {
            id: Uuid::now_v7(),
            ts: Utc::now(),
            lifecycle: LifecycleState::Calibrating,
            reason: Some("roi_update".to_string()),
            vlm_diff: None,
            roi_changes: Some(json!({"zone_id": zone_id.to_string(), "lines": applied})),
            cameras_ok: None,
            cameras_total: None,
            csi_nodes_ok: None,
            csi_nodes_total: None,
        })
        .await?;
    Ok(Json(
        json!({"zone_id": zone_id.to_string(), "applied": true, "lines": applied}),
    ))
}

// ------------------- counts / hourly -------------------

#[derive(Deserialize)]
struct CountsQuery {
    from: String,
    to: String,
}

async fn get_counts(
    State(state): State<Shared>,
    Path(zone_id): Path<String>,
    Query(q): Query<CountsQuery>,
) -> Result<Json<Value>, ApiError> {
    let zone = ZoneId::try_new(&zone_id).map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    let from = DateTime::parse_from_rfc3339(&q.from)
        .map_err(|e| ApiError::BadRequest(format!("from: {e}")))?
        .with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339(&q.to)
        .map_err(|e| ApiError::BadRequest(format!("to: {e}")))?
        .with_timezone(&Utc);
    let events = state.repos.count_events(&zone, from, to).await?;
    let agg = aggregate(zone.as_str(), from, to, &events);
    Ok(Json(
        serde_json::to_value(&agg).map_err(|e| ApiError::Internal(format!("{e}")))?,
    ))
}

#[derive(Deserialize)]
struct HourQuery {
    date: String,
}

async fn get_hourly(
    State(state): State<Shared>,
    Path(zone_id): Path<String>,
    Query(q): Query<HourQuery>,
) -> Result<Json<Value>, ApiError> {
    let date = NaiveDate::parse_from_str(&q.date, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("date: {e}")))?;
    let day_start = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| ApiError::BadRequest("bad date".to_string()))?
        .and_utc();
    let day_end = day_start + chrono::Duration::hours(24);
    let zone = ZoneId::try_new(&zone_id).map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    let events = state.repos.count_events(&zone, day_start, day_end).await?;
    let curve = hourly_curve((date.year(), date.month(), date.day()), &events);
    Ok(Json(
        json!({"zone_id": zone.to_string(), "date": date.to_string(), "hours": curve}),
    ))
}

// ------------------- audit corrections -------------------

#[derive(Deserialize)]
struct CorrectionSubmit {
    zone_id: String,
    reported_count: i64,
    actual_count: i64,
    ts_from: chrono::DateTime<Utc>,
    ts_to: chrono::DateTime<Utc>,
    failure_tags: u16,
    notes: Option<String>,
    operator_id: String,
    wants_retrain: Option<bool>,
}

async fn post_correction(
    State(state): State<Shared>,
    Json(sub): Json<CorrectionSubmit>,
) -> Result<Json<Value>, ApiError> {
    let operator = OperatorId::try_new(&sub.operator_id)
        .map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    let feedback = HumanAuditFeedback::try_new(
        ZoneId::try_new(&sub.zone_id).map_err(|e| ApiError::BadRequest(format!("{e:?}")))?,
        sub.reported_count,
        sub.actual_count,
        sub.ts_from,
        sub.ts_to,
        FailureTag::from_db_mask(sub.failure_tags as i16),
        sub.notes,
        operator,
        sub.wants_retrain.unwrap_or(false),
    )
    .map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    let delta = feedback.actual_count - feedback.reported_count;
    let reward =
        rl_controller::reward::correction_reward(feedback.reported_count, feedback.actual_count);
    let _ = delta;
    let row = AuditCorrectionRow {
        correction_id: Uuid::now_v7(),
        feedback,
        outcome: CorrectionOutcome::Submitted,
        reward_computed: Some(reward),
        root_cause_report: None,
        created_at: Utc::now(),
    };
    let id = row.correction_id;
    let retrain = row.feedback.wants_retrain;
    state.repos.put_correction(row).await?;
    Ok(Json(
        json!({"correction_id": id, "reward_computed": reward, "retrain": retrain}),
    ))
}

#[derive(Deserialize)]
struct CommitReq {
    confirm: Option<bool>,
}

async fn commit_correction(
    State(state): State<Shared>,
    Path(id): Path<Uuid>,
    Json(req): Json<CommitReq>,
) -> Result<Json<Value>, ApiError> {
    let row = state
        .repos
        .get_correction(id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("correction {id}")))?;
    let delta = (row.feedback.actual_count - row.feedback.reported_count).abs();
    if delta > BIG_DELTA_CONFIRM && req.confirm != Some(true) {
        return Err(ApiError::Conflict(format!(
            "delta {delta} exceeds {BIG_DELTA_CONFIRM}; re-post with confirm:true"
        )));
    }
    if row.outcome == CorrectionOutcome::Committed {
        return Ok(Json(
            json!({"correction_id": id, "outcome": "committed", "idempotent": true}),
        ));
    }
    state
        .repos
        .set_correction_outcome(id, CorrectionOutcome::Committed)
        .await?;
    Ok(Json(json!({
        "correction_id": id,
        "outcome": "committed",
        "reward_computed": row.reward_computed,
        "retrain": row.feedback.wants_retrain,
    })))
}

async fn get_pending(State(state): State<Shared>) -> Result<Json<Value>, ApiError> {
    let rows = state.repos.pending_corrections().await?;
    Ok(Json(json!({"pending": rows})))
}

// ------------------- RL surface -------------------

async fn get_rl_state(State(state): State<Shared>) -> impl IntoResponse {
    let view = state.rl().clone();
    Json(json!({
        "paused_until": view.paused_until,
        "override_note": view.override_note,
        "rejected_24h": view.rejected_24h,
    }))
}

#[derive(Deserialize)]
struct OverrideReq {
    until: chrono::DateTime<Utc>,
    note: Option<String>,
}

async fn post_rl_override(
    State(state): State<Shared>,
    Json(req): Json<OverrideReq>,
) -> Result<Json<Value>, ApiError> {
    let now = Utc::now();
    if req.until <= now {
        return Err(ApiError::BadRequest(
            "override must end in the future".to_string(),
        ));
    }
    let note = req.note.clone().unwrap_or_else(|| "manual override".into());
    {
        let mut view = state.rl();
        view.paused_until = Some(req.until);
        view.override_note = Some(note.clone());
    }
    state.hub.push(WsEvent::RlTransition {
        before: RlAction::defaults(),
        after: RlAction::defaults(),
        reason: note.clone(),
        was_rejected: false,
    });
    state
        .repos
        .put_rl_transition(RlTransitionRow {
            transition_id: Uuid::now_v7(),
            ts: now,
            zone_id: None,
            before: RlAction::defaults(),
            after: RlAction::defaults(),
            reward: None,
            was_clamped: false,
            rejected: false,
            reason: RlReason::ManualOverride,
        })
        .await?;
    Ok(Json(
        json!({"ok": true, "paused_until": req.until, "note": note}),
    ))
}

#[derive(Deserialize)]
struct HistoryQuery {
    date: String,
}

async fn get_rl_history(
    State(state): State<Shared>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let date = NaiveDate::parse_from_str(&q.date, "%Y-%m-%d")
        .map_err(|e| ApiError::BadRequest(format!("date: {e}")))?;
    let rows = state
        .repos
        .rl_history((date.year(), date.month(), date.day()))
        .await?;
    Ok(Json(json!({"date": date.to_string(), "transitions": rows})))
}

// ------------------- snapshots / labeling -------------------

#[derive(Deserialize)]
struct SnapshotQueryBody {
    zone_id: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    only_flagged: Option<bool>,
    limit: Option<usize>,
}

async fn query_snapshots(
    State(state): State<Shared>,
    Json(body): Json<SnapshotQueryBody>,
) -> Result<Json<Value>, ApiError> {
    let zone = body
        .zone_id
        .as_ref()
        .map(|z| ZoneId::try_new(z).map_err(|e| ApiError::BadRequest(format!("{e:?}"))))
        .transpose()?;
    let rows = state
        .repos
        .query_snapshots(
            zone.as_ref(),
            body.from,
            body.to,
            body.only_flagged.unwrap_or(false),
        )
        .await?;
    let cap = body.limit.unwrap_or(100).min(500);
    let public: Vec<Value> = rows
        .iter()
        .take(cap)
        .map(|r| {
            json!({
                "frame_id": r.frame_id, "zone_id": r.zone_id, "ts": r.ts,
                "camera_id": r.camera_id, "uncertainty_mask": r.uncertainty_mask,
                "divergence": r.divergence, "promotion": r.promotion,
                "has_coco": r.coco_annotations.is_some(),
                "reviewed_by": r.reviewed_by,
            })
        })
        .collect();
    Ok(Json(json!({"snapshots": public, "total": rows.len()})))
}

async fn get_label_queue(State(state): State<Shared>) -> Result<Json<Value>, ApiError> {
    let rows = state.repos.query_snapshots(None, None, None, true).await?;
    let queue: Vec<Value> = rows
        .iter()
        .filter(|r| {
            r.promotion == SnapshotPromotion::Amber || r.promotion == SnapshotPromotion::Uploaded
        })
        .map(|r| {
            json!({
                "frame_id": r.frame_id, "zone_id": r.zone_id, "ts": r.ts,
                "camera_id": r.camera_id, "divergence": r.divergence,
                "uncertainty_mask": r.uncertainty_mask, "promotion": r.promotion,
            })
        })
        .collect();
    Ok(Json(json!({"queue": queue})))
}

#[derive(Deserialize)]
struct LabelApprove {
    operator_id: String,
    #[serde(default)]
    coco_annotations: Option<Value>,
}

async fn approve_label(
    State(state): State<Shared>,
    Path(frame_id): Path<Uuid>,
    Json(sub): Json<LabelApprove>,
) -> Result<Json<Value>, ApiError> {
    let operator = OperatorId::try_new(&sub.operator_id)
        .map_err(|e| ApiError::BadRequest(format!("{e:?}")))?;
    state
        .repos
        .review_snapshot(frame_id, operator.clone(), sub.coco_annotations)
        .await?;
    // A reviewed frame leaves the auto-label queue on its next promotion
    // cycle; LabeledOk is the terminal reviewed state (docs/09 §3).
    state
        .repos
        .set_snapshot_promotion(frame_id, SnapshotPromotion::LabeledOk)
        .await?;
    Ok(Json(json!({
        "frame_id": frame_id,
        "approved": true,
        "reviewed_by": sub.operator_id,
    })))
}

// ------------------- websocket -------------------

async fn ws_live(
    State(state): State<Shared>,
    Query(params): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let raw = params.get("topics").map(String::as_str).unwrap_or("");
    let topics = match parse_topics(raw) {
        Ok(topics) => topics,
        Err(msg) => return ApiError::BadRequest(msg).into_response(),
    };
    let rx = state.hub.receiver();
    upgrade.on_upgrade(move |socket| pump(socket, rx, topics))
}

async fn pump(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<WsEvent>,
    topics: Option<HashSet<&'static str>>,
) {
    loop {
        let event = match rx.recv().await {
            Ok(event) => event,
            // Lagged means this client is slow: drop it (docs/10 §3).
            Err(_) => break,
        };
        if !topics_matches(&topics, event.topic()) {
            continue;
        }
        let Some(wire) = to_wire(&event) else {
            break;
        };
        let Ok(text) = serde_json::to_string(&wire) else {
            break;
        };
        // Socket closed/buffer full: stop the pump; client reconnects.
        if socket.send(Message::Text(text.into())).await.is_err() {
            break;
        }
    }
}

fn topics_matches(topics: &Option<HashSet<&'static str>>, topic: &str) -> bool {
    match topics {
        Some(set) => set.contains(topic),
        None => true,
    }
}
