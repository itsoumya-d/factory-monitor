//! HTTP-level tests through the real router: every documented route's
//! happy path and its validation errors, on the in-memory repo. No
//! network: `tower::ServiceExt::oneshot` drives `Router` directly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use api_server::config::Config;
use api_server::repo::{InMemoryRepo, Repos, SnapshotPromotion, SnapshotRow, Zone};
use api_server::routes::build_router;
use api_server::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, TimeZone, Utc};
use common_types::bounds::DetectorThresholds;
use common_types::counting::{PackageCountEvent, UncertaintyFlags};
use common_types::fusion::ModalityBlend;
use common_types::ids::{CameraId, ModelVersion, TrackId, ZoneId};
use tower::ServiceExt;
use uuid::Uuid;

fn ts(h: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 7, h, 0, 0)
        .latest()
        .unwrap_or_else(Utc::now)
}

fn body(raw: &str) -> Body {
    Body::from(raw.to_owned())
}

fn get(path: &str) -> Request<String> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(String::new())
        .expect("request builder")
}

fn post(path: &str) -> Request<String> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(String::new())
        .expect("request builder")
}

struct Harness {
    router: Router,
    repos: Arc<InMemoryRepo>,
}

fn harness() -> Harness {
    let repos = Arc::new(InMemoryRepo::new());
    let state = Arc::new(AppState::new(Config::default(), repos.clone()));
    Harness {
        router: build_router(state),
        repos,
    }
}

/// JSON-shaped request/response pair.
async fn call_json(
    router: &Router,
    req: Request<String>,
    body: Body,
) -> (StatusCode, serde_json::Value) {
    let req = req.map(|_old| body);
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body read");
    (
        status,
        serde_json::from_slice(&bytes).expect("jsonable body"),
    )
}

async fn call_text(router: &Router, req: Request<String>) -> (StatusCode, String) {
    let req = req.map(|_old| String::new());
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("body read");
    (status, String::from_utf8(bytes.to_vec()).expect("utf8"))
}

fn counting(zone: &str, h: u32, delta: i64, flagged: bool) -> PackageCountEvent {
    PackageCountEvent::try_new(
        ts(h),
        ts(h) + Duration::minutes(5),
        ZoneId::try_new(zone).expect("zone"),
        delta,
        delta,
        0.92,
        if flagged {
            UncertaintyFlags::OCCLUDED_ESTIMATE
        } else {
            UncertaintyFlags::empty()
        },
        ModalityBlend::normalize(0.7),
        DetectorThresholds::try_new(0.45, 0.45, 45).expect("th"),
        vec![TrackId::new(7)],
        vec![],
        ModelVersion::try_new("hermetic").expect("mv"),
    )
    .expect("event")
}

#[tokio::test]
async fn health_ready_and_metrics_pipeline() {
    let h = harness();
    let (code, json) = call_json(&h.router, get("/healthz"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["status"], "ok");

    let (code, _) = call_json(&h.router, get("/readyz"), body("")).await;
    assert_eq!(code, StatusCode::OK);

    // metrics: the counter for /healthz was bumped by the calls above.
    let (code, text) = call_text(&h.router, get("/metrics")).await;
    assert_eq!(code, StatusCode::OK);
    assert!(
        text.contains("shado_requests_total{route=\"/healthz\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("shado_requests_total{route=\"other\"}"),
        "{text}"
    );
    assert!(text.contains("shado_ws_clients"), "{text}");
    assert!(text.contains("shado_uptime_seconds"), "{text}");
}

#[tokio::test]
async fn lifecycle_matrix_is_enforced() {
    let h = harness();
    let (code, json) = call_json(&h.router, get("/api/v1/lifecycle"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["state"], "Standby");

    // Standby -> Counting violates the docs/02 §7 matrix.
    let (code, _) = call_json(
        &h.router,
        post("/api/v1/lifecycle/force"),
        body("{\"to\":\"Counting\"}"),
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT);

    let (code, json) = call_json(
        &h.router,
        post("/api/v1/lifecycle/force"),
        body("{\"to\":\"Calibrating\",\"reason\":\"morning\"}"),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["to"], "Calibrating");

    // Calibration done: Calibrating -> Counting allowed.
    let (code, _) = call_json(
        &h.router,
        post("/api/v1/lifecycle/force"),
        body("{\"to\":\"Counting\"}"),
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    // Unknown state -> 400.
    let (code, _) = call_json(
        &h.router,
        post("/api/v1/lifecycle/force"),
        body("{\"to\":\"Coffee\"}"),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn counts_and_hourly_aggregates_hermetic() {
    let h = harness();
    h.repos
        .put_zone(
            Zone::try_new(ZoneId::try_new("line_1").expect("z"), "Line 1".to_owned())
                .expect("zone"),
        )
        .await
        .expect("seed");
    h.repos
        .insert_count_event(counting("line_1", 14, 10, false))
        .await
        .expect("e1");
    h.repos
        .insert_count_event(counting("line_1", 14, 4, false))
        .await
        .expect("e2");
    h.repos
        .insert_count_event(counting("line_1", 15, 3, true))
        .await
        .expect("e3");

    let (code, json) = call_json(
        &h.router,
        get("/api/v1/zones/line_1/counts?from=2026-04-07T00:00:00Z&to=2026-04-07T23:00:00Z"),
        body(""),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 17, "{json}");
    assert_eq!(json["events"], 3);
    assert_eq!(json["occluded_events"], 1);
    assert!((json["avg_confidence"].as_f64().unwrap_or(0.0) - 0.92).abs() < 1e-4);

    // Reversed window is a client error.
    let (code, _) = call_json(
        &h.router,
        get("/api/v1/zones/line_1/counts?from=2026-04-07T05:00:00Z&to=2026-04-07T00:00:00Z"),
        body(""),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    let (code, json) = call_json(
        &h.router,
        get("/api/v1/zones/line_1/hourly?date=2026-04-07"),
        body(""),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    let packages: Vec<i64> = json["hours"]
        .as_array()
        .expect("hours array")
        .iter()
        .map(|pt| pt["packages"].as_i64().unwrap_or(0))
        .collect();
    assert_eq!(packages, vec![14, 3], "14:00 = 10+4, 15:00 = 3 (occluded)");
    assert_eq!(json["hours"][1]["occluded_events"], 1);

    let (code, _) = call_json(
        &h.router,
        get("/api/v1/zones/line_1/hourly?date=bogus"),
        body(""),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rois_then_calibration_surface() {
    let h = harness();
    h.repos
        .put_zone(
            Zone::try_new(ZoneId::try_new("line_2").expect("z"), "Line 2".to_owned())
                .expect("zone"),
        )
        .await
        .expect("seed");
    // Calibration surface before any snapshot: 404.
    let (code, _) = call_json(&h.router, get("/api/v1/calibration/latest"), body("")).await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    let (code, json) = call_json(
        &h.router,
        post("/api/v1/calibration/rois/line_2"),
        body("{\"lines\": [ [ [10,10], [300,410] ], [ [500,11], [640,212] ] ]}"),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["applied"], true);
    assert_eq!(json["lines"], 2);

    // One-point line is rejected.
    let (code, _) = call_json(
        &h.router,
        post("/api/v1/calibration/rois/line_2"),
        body("{\"lines\": [ [[1,1]] ]}"),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    // Upshot: EXISTS with rois recorded in the latest snapshot.
    let (code, json) = call_json(&h.router, get("/api/v1/calibration/latest"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["roi_changes"]["zone_id"], "line_2");
    assert_eq!(json["roi_changes"]["lines"], 2);
}

#[tokio::test]
async fn corrections_flow_with_commit_confirmation() {
    let h = harness();
    let payload = r#"{"zone_id":"line_1","reported_count":100,"actual_count":250,
        "ts_from":"2026-04-07T14:00:00Z","ts_to":"2026-04-07T15:00:00Z",
        "failure_tags":1,"notes":"missed crates","operator_id":"op1","wants_retrain":true}"#;
    let (code, json) = call_json(&h.router, post("/api/v1/audit/corrections"), body(payload)).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    let id = json["correction_id"].as_str().expect("uuid present");
    assert!((json["reward_computed"].as_f64().unwrap_or(0.0) - (-1.5)).abs() < 1e-6);
    assert_eq!(json["retrain"], true);

    let (code, json) = call_json(&h.router, get("/api/v1/audit/pending"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["pending"].as_array().map(Vec::len).unwrap_or(0), 1);

    let commit_url = format!("/api/v1/audit/corrections/{id}/commit");
    // delta 150 > BIG_DELTA_CONFIRM=100 without confirmation: 409.
    let (code, _) = call_json(&h.router, post(&commit_url), body("{\"confirm\":false}")).await;
    assert_eq!(code, StatusCode::CONFLICT);

    let (code, json) = call_json(&h.router, post(&commit_url), body("{\"confirm\":true}")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["outcome"], "committed");

    // Post-commit second call is idempotent, not an error.
    let (code, json) = call_json(&h.router, post(&commit_url), body("{\"confirm\":true}")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["idempotent"], true);

    // Unknown id is a 404.
    let (code, _) = call_json(
        &h.router,
        post(&format!(
            "/api/v1/audit/corrections/{}/commit",
            Uuid::now_v7()
        )),
        body("{\"confirm\":true}"),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rl_state_override_and_history() {
    let h = harness();
    let (code, json) = call_json(&h.router, get("/api/v1/rl/state"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert!(json["paused_until"].is_null());

    // Past-dated override is rejected.
    let (code, _) = call_json(
        &h.router,
        post("/api/v1/rl/override"),
        body("{\"until\":\"2020-01-01T00:00:00Z\",\"note\":\"stale\"}"),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    // until is relative to wall-clock today (CI clocks drift between environments).
    let until = Utc::now() + Duration::hours(24);
    let until_json = serde_json::to_value(until).expect("until json");
    let note = "bands off for inventory";
    let payload = format!("{{\"until\":{until_json},\"note\":\"{note}\"}}");
    let (code, json) = call_json(&h.router, post("/api/v1/rl/override"), body(&payload)).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    assert_eq!(json["ok"], true);

    let (code, json) = call_json(&h.router, get("/api/v1/rl/state"), body("")).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(json["paused_until"].as_str(), until_json.as_str(), "{json}");

    // History for today carries the manual_override transition.
    let today = Utc::now().date_naive().to_string();
    let (code, json) = call_json(
        &h.router,
        get(&format!("/api/v1/rl/history?date={today}")),
        body(""),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    let transitions = json["transitions"].as_array().expect("transitions list");
    assert!(
        transitions.iter().any(|t| t["reason"] == "manual_override"),
        "{json}"
    );
}

#[tokio::test]
async fn snapshots_query_and_label_queue() {
    let h = harness();
    let flagged = SnapshotRow {
        frame_id: Uuid::now_v7(),
        zone_id: ZoneId::try_new("line_1").expect("z"),
        ts: ts(8),
        camera_id: CameraId::try_new("camA").expect("c"),
        jpeg_lowres: Some(vec![0xFF, 0xD8]),
        uncertainty_mask: 1,
        divergence: Some(0.42),
        state: serde_json::json!({}),
        promotion: SnapshotPromotion::Amber,
        vlm_root_cause: None,
        coco_annotations: None,
        reviewed_by: None,
        reviewed_at: None,
    };
    let clean = SnapshotRow {
        frame_id: Uuid::now_v7(),
        zone_id: ZoneId::try_new("line_1").expect("z"),
        ts: ts(12),
        camera_id: CameraId::try_new("camA").expect("c"),
        jpeg_lowres: None,
        uncertainty_mask: 0,
        divergence: Some(0.02),
        state: serde_json::json!({}),
        promotion: SnapshotPromotion::Amber,
        vlm_root_cause: None,
        coco_annotations: None,
        reviewed_by: None,
        reviewed_at: None,
    };
    h.repos.put_snapshot(flagged.clone()).await.expect("snap f");
    h.repos.put_snapshot(clean).await.expect("snap c");

    // Only the flagged row passes the pre-filter.
    let (code, json) = call_json(
        &h.router,
        post("/api/v1/snapshots/query"),
        body("{\"zone_id\":\"line_1\",\"only_flagged\":true}"),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{json}");
    let snaps = json["snapshots"].as_array().expect("snaps");
    assert_eq!(snaps.len(), 1);
    let frame_id = flagged.frame_id.to_string();
    assert_eq!(
        snaps[0]["frame_id"].as_str(),
        Some(frame_id.as_str()),
        "{json}"
    );
    assert!(
        snaps[0].get("jpeg_lowres").is_none(),
        "image bytes never leak"
    );

    // Unfiltered query returns both, capped rows only.
    let (code, json) = call_json(
        &h.router,
        post("/api/v1/snapshots/query"),
        body("{\"limit\":1}"),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(json["snapshots"].as_array().map(Vec::len).unwrap_or(9), 1);
    assert_eq!(json["total"], 2);

    // The label queue shows only Amber/Uploaded rows.
    let (code, json) = call_json(&h.router, get("/api/v1/labeling/queue"), body("")).await;
    assert_eq!(code, StatusCode::OK, "{json}");
    let queue = json["queue"].as_array().expect("queue");
    assert_eq!(queue.len(), 1, "only the flagged amber row is queued");
    assert_eq!(queue[0]["frame_id"].as_str(), Some(frame_id.as_str()));
}

#[tokio::test]
async fn ws_topics_reject_unknowns() {
    let h = harness();
    // Without an upgrade handshake the extractor rejects with 4xx first;
    // the real topic validation is covered in ws.rs unit tests.
    let (code, _) = call_text(&h.router, get("/ws/live?topics=junk")).await;
    assert!(
        code.is_client_error(),
        "bad topics expected 4xx, got {code}"
    );

    let (code, _) = call_text(&h.router, get("/ws/live?topics=,,")).await;
    assert!(
        code.is_client_error(),
        "blank topics expected 4xx, got {code}"
    );
}
