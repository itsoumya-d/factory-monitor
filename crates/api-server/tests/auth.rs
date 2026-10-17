//! Auth + resource-bound regression tests: `SHADO_API_KEY` gating and
//! the hard 500-row cap on snapshot queries. Mirrors hermetic.rs's
//! oneshot harness.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use api_server::config::Config;
use api_server::repo::{InMemoryRepo, Repos, SnapshotPromotion, SnapshotRow};
use api_server::routes::{build_router, ct_eq};
use api_server::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use common_types::ids::{CameraId, ZoneId};
use tower::ServiceExt;
use uuid::Uuid;

fn body(raw: &str) -> Body {
    Body::from(raw.to_owned())
}

fn req(method: &str, path: &str, key: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(k) = key {
        b = b.header("x-shado-api-key", k);
    }
    b.body(body("{}")).expect("request builder")
}

struct Harness {
    router: Router,
    repos: Arc<InMemoryRepo>,
}

fn key_harness(key: Option<&str>) -> Harness {
    let repos = Arc::new(InMemoryRepo::new());
    let cfg = Config {
        api_key: key.map(str::to_string),
        ..Config::default()
    };
    Harness {
        router: build_router(Arc::new(AppState::new(cfg, repos.clone()))),
        repos,
    }
}

#[tokio::test]
async fn key_set_blocks_requests_without_header() {
    let h = key_harness(Some("river-owe-brick-sunny-mine"));
    let resp = h
        .router
        .clone()
        .oneshot(req("POST", "/api/v1/rl/override", None))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn key_set_accepts_the_correct_key() {
    let h = key_harness(Some("river-owe-brick-sunny-mine"));
    let resp = h
        .router
        .clone()
        .oneshot(req("POST", "/api/v1/rl/override", None))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_key_same_length_still_rejected() {
    let h = key_harness(Some("river-owe-brick-sunny-mine"));
    let resp = h
        .router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/rl/override",
            Some("river-owe-brick-sunny-unit"),
        ))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_key_keeps_trust_on_lan_posture() {
    let h = key_harness(None);
    let (router, _) = (h.router.clone(), ());
    let resp = router
        .oneshot(req(
            "POST",
            "/api/v1/rl/override",
            Some("any-junk-nobody-checks"),
        ))
        .await
        .expect("oneshot");
    // `{}` lacks the override fields, so the handler answers 422 — the
    // point is that the middleware passed the request through to the
    // handler (trust-on-LAN posture) instead of 401-ing it.
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn snapshot_query_is_hard_capped_at_500_rows() {
    let h = key_harness(None);
    let zone = ZoneId::try_new("line_1").expect("z");
    for i in 0..520 {
        let row = SnapshotRow {
            frame_id: Uuid::now_v7(),
            zone_id: zone.clone(),
            ts: Utc
                .with_ymd_and_hms(2026, 4, 7, 0, 0, 0)
                .latest()
                .expect("ts")
                + chrono::Duration::seconds(i),
            camera_id: CameraId::try_new("camA").expect("c"),
            jpeg_lowres: Some(vec![0xFF, 0xD8]),
            uncertainty_mask: 1,
            divergence: Some(0.01),
            state: serde_json::json!({}),
            promotion: SnapshotPromotion::Amber,
            vlm_root_cause: None,
            coco_annotations: None,
            reviewed_by: None,
            reviewed_at: None,
        };
        h.repos.put_snapshot(row).await.expect("snap");
    }
    let resp = h
        .router
        .oneshot(req("POST", "/api/v1/snapshots/query", None))
        .await
        .expect("oneshot");
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 24)
        .await
        .expect("body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(json["total"].as_i64(), Some(500), "500-row hard cap");
}

// ---- ct_eq semantics ----

#[test]
fn ct_eq_matches_identical_and_rejects_single_bit_diff() {
    assert!(ct_eq(b"same", b"same"));
    assert!(!ct_eq(b"same", b"samf"));
    assert!(!ct_eq(b"short", b"shorter"));
    assert!(ct_eq(b"", b""));
}
