//! The orchestrator: only component allowed to call the cloud (docs/09).
//! Every op runs the same gauntlet: breaker gate → payload caps → budget
//! preflight → provider call with 1/4/16 s retries → schema validation →
//! parse → budget charge → metrics.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use serde_json::Value;

use crate::auto_label::{CocoDataset, LabelDecision, LabelItem, acceptance_gate, package_coco};
use crate::breaker::{CircuitBreaker, VlmMetrics};
use crate::error::VlmError;
use crate::guardrails::{DailyBudget, OpKind, retry_backoff};
use crate::layout_diff::{self, Floorplan, LayoutChangePlan, MorningBlock};
use crate::payload::PayloadCaps;
use crate::provider::{VlmProvider, VlmRequest};
use crate::root_cause::{self, CulpritEpisode, RootCauseReport};

const MAX_ATTEMPTS: u32 = 3;

#[async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, d: std::time::Duration);
}

/// Production sleeper.
pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, d: std::time::Duration) {
        tokio::time::sleep(d).await;
    }
}

/// Test sleeper: never blocks.
pub struct NoSleep;

#[async_trait]
impl Sleeper for NoSleep {
    async fn sleep(&self, _d: std::time::Duration) {}
}

pub struct VlmOrchestrator {
    provider: Arc<dyn VlmProvider>,
    breaker: Mutex<CircuitBreaker>,
    budget: Mutex<DailyBudget>,
    caps: PayloadCaps,
    metrics: VlmMetrics,
    sleeper: Arc<dyn Sleeper>,
    max_attempts: u32,
}

pub struct OrchestratorDeps {
    pub provider: Arc<dyn VlmProvider>,
    pub budget: DailyBudget,
    pub caps: PayloadCaps,
    pub sleeper: Arc<dyn Sleeper>,
}

impl VlmOrchestrator {
    pub fn new(deps: OrchestratorDeps) -> Self {
        Self {
            provider: deps.provider,
            breaker: Mutex::new(CircuitBreaker::default()),
            budget: Mutex::new(deps.budget),
            caps: deps.caps,
            metrics: VlmMetrics::default(),
            sleeper: deps.sleeper,
            max_attempts: MAX_ATTEMPTS,
        }
    }

    pub fn metrics(&self) -> &VlmMetrics {
        &self.metrics
    }

    pub fn budget_used_usd(&self) -> f64 {
        self.budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .used_usd()
    }

    /// Morning diffing op (docs/09 §2): validate the frame crops against
    /// the payload caps first, then prompt, then parse + auto-apply.
    pub async fn morning_diff(
        &self,
        blocks: &[MorningBlock],
        yesterday_b64: &str,
        zones: &[String],
        floorplan: &Floorplan,
    ) -> Result<LayoutChangePlan, VlmError> {
        for b in blocks {
            let crop = base64::engine::general_purpose::STANDARD
                .decode(&b.crop_b64)
                .map_err(|e| VlmError::InvalidPayload(format!("crop b64: {e}")))?;
            self.caps.check_image_jpeg(&crop)?;
        }
        let yesterday = base64::engine::general_purpose::STANDARD
            .decode(yesterday_b64)
            .map_err(|e| VlmError::InvalidPayload(format!("baseline b64: {e}")))?;
        self.caps.check_image_jpeg(&yesterday)?;

        let req = layout_diff::build_morning_request(blocks, yesterday_b64, zones);
        let schema = layout_diff::expected_schema();
        let value = self.execute(OpKind::MorningDiff, req, &schema).await?;
        layout_diff::parse_layout_response(&value, floorplan)
    }

    /// EOD root-cause op (docs/09 §3). `None` = clean day, no culprit
    /// windows, no cloud call.
    pub async fn eod_root_cause(
        &self,
        episodes: &[CulpritEpisode],
        predicted: i64,
        actual: i64,
        known_tags: &[String],
    ) -> Result<Option<RootCauseReport>, VlmError> {
        if episodes.is_empty() {
            return Ok(None);
        }
        let req = root_cause::build_root_cause_request(predicted, actual, known_tags, episodes)?;
        for ep in episodes {
            for f in &ep.frames {
                let frame = base64::engine::general_purpose::STANDARD
                    .decode(&f.frame_b64)
                    .map_err(|e| VlmError::InvalidPayload(format!("frame b64: {e}")))?;
                self.caps.check_image_jpeg(&frame)?;
            }
        }
        let schema = root_cause::expected_schema();
        let value = self.execute(OpKind::Eod, req, &schema).await?;
        Ok(Some(root_cause::parse_root_cause_response(&value)?))
    }

    /// Auto-label pass (docs/09 §4): gates pending items and packages
    /// COCO. Labels originate in root-cause corrections; no cloud call.
    pub fn collect_coco(&self, items: &[LabelItem], categories: &[String]) -> CocoDataset {
        package_coco(items, categories)
    }

    pub fn label_decision(
        &self,
        confidence: f32,
        from_known_root: bool,
        days_since_rollout: u32,
    ) -> LabelDecision {
        acceptance_gate(confidence, from_known_root, days_since_rollout)
    }

    /// The gauntlet shared by all cloud ops.
    async fn execute(
        &self,
        op: OpKind,
        req: VlmRequest,
        schema: &crate::schema::JsonSchemaSpec,
    ) -> Result<Value, VlmError> {
        // Circuit gate (docs/09 §1).
        self.breaker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .check(Instant::now())?;
        // Budget gate (docs/09 §6): over cap ⇒ soft mode.
        crate::guardrails::preflight(
            op,
            &self.budget.lock().unwrap_or_else(|e| e.into_inner()),
            Utc::now(),
        )?;
        // Prompt sanity: empty or oversized prompt means caller wiring broke.
        if req.wire_bytes() == 0 {
            return Err(VlmError::InvalidPayload("empty prompt".to_string()));
        }
        self.validate_prompt_size(&req)?;

        let started = Instant::now();
        let mut last_err = None;
        for attempt in 0..self.max_attempts {
            self.metrics.record_call();
            match self.provider.chat_json(req.clone(), schema).await {
                Ok(value) => {
                    self.breaker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record_success();
                    self.metrics
                        .record_latency_ms(started.elapsed().as_millis() as u64);
                    let cost =
                        DailyBudget::estimate_cost_usd(req.wire_bytes(), value.to_string().len());
                    self.budget
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .charge(cost, Utc::now());
                    return Ok(value);
                }
                Err(err) => {
                    self.breaker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record_failure(Instant::now());
                    self.metrics.record_failure();
                    let retryable = err.is_retryable();
                    last_err = Some(err);
                    if retryable && attempt + 1 < self.max_attempts {
                        self.metrics.record_retry();
                        drop_wait(&self.sleeper, retry_backoff(attempt)).await;
                    } else {
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap_or(VlmError::Http("no provider attempts".into())))
    }

    /// Payload policy (docs/09 §6): keep a hard ceiling on the serialized
    /// prompt; per-image caps are enforced by the ops above.
    fn validate_prompt_size(&self, req: &VlmRequest) -> Result<(), VlmError> {
        let body =
            serde_json::to_vec(&req.body).map_err(|e| VlmError::InvalidPayload(e.to_string()))?;
        if body.len() > self.caps.max_prompt_bytes {
            return Err(VlmError::InvalidPayload(format!(
                "prompt body {0} bytes over cap {1}",
                body.len(),
                self.caps.max_prompt_bytes
            )));
        }
        Ok(())
    }
}

async fn drop_wait(sleeper: &Arc<dyn Sleeper>, wait: std::time::Duration) {
    sleeper.sleep(wait).await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::guardrails::{DEFAULT_DAILY_USD_CAP, OpKind};
    use crate::layout_diff::{RoiFix, TASK_LAYOUT_DIFF};
    use crate::provider::MockProvider;
    use crate::testutil;
    use std::collections::BTreeMap;

    fn jpg(svg_w: u32, h: u32) -> String {
        crate::payload::encode_b64(&testutil::jpeg_bytes(svg_w, h))
    }

    fn layout_answer() -> Value {
        serde_json::json!({
            "detect_changes": [
                {"kind": "moved", "object": "conveyor_belt", "zone_id": "line_1",
                 "blocking_counting_line": true,
                 "suggested_roi_fix": {"zone_id": "line_1", "new_line": [[10.0, 100.0], [290.0, 100.0]]}}
            ],
            "walkway_blocked": false,
            "new_equipment_needs_roi": []
        })
    }

    fn floorplan() -> Floorplan {
        let mut zones = BTreeMap::new();
        zones.insert("line_1".to_string(), [0.0, 0.0, 300.0, 200.0]);
        Floorplan::new(zones)
    }

    fn orch(provider: MockProvider, cap: f64) -> VlmOrchestrator {
        VlmOrchestrator::new(OrchestratorDeps {
            provider: Arc::new(provider),
            budget: DailyBudget::new(cap.max(DEFAULT_DAILY_USD_CAP)),
            caps: PayloadCaps::default(),
            sleeper: Arc::new(NoSleep),
        })
    }

    #[tokio::test]
    async fn morning_diff_end_to_end_charges_and_metrics() {
        let mock = MockProvider::new("mock");
        mock.respond_with(TASK_LAYOUT_DIFF, layout_answer());
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let blocks = vec![MorningBlock {
            block_id: "r1c1".into(),
            ssim_delta: -0.4,
            bbox_2d: [0.0; 4],
            crop_b64: jpg(320, 240),
        }];
        let zones = vec!["line_1".to_string()];
        let plan = o
            .morning_diff(&blocks, &jpg(640, 480), &zones, &floorplan())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.auto_applied.len(), 1);
        assert_eq!(
            plan.auto_applied[0],
            RoiFix {
                zone_id: "line_1".into(),
                new_line: vec![[10.0, 100.0], [290.0, 100.0]],
            }
        );
        assert!(o.budget_used_usd() > 0.0, "successful call is billed");
        let m = o.metrics().snapshot();
        assert_eq!(m.calls_total, 1);
        assert_eq!(m.failures_total, 0);
        assert!(m.latency_calls >= 1);
    }

    #[tokio::test]
    async fn retries_the_retryable_and_succeeds() {
        let mock = MockProvider::new("mock");
        mock.plan(vec![
            Err(VlmError::RateLimited {
                provider: "m".into(),
            }),
            Err(VlmError::Provider {
                provider: "m".into(),
                status: 500,
                body: "t".into(),
            }),
            Ok(layout_answer()),
        ]);
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let plan = o
            .morning_diff(&[], &jpg(320, 240), &["line_1".to_string()], &floorplan())
            .await
            .unwrap_or_else(|e| panic!("flaky provider must still succeed: {e}"));
        assert_eq!(plan.changes.len(), 1, "third attempt answered");
        assert_eq!(plan.auto_applied[0].zone_id, "line_1");
        let m = o.metrics().snapshot();
        assert_eq!(m.calls_total, 3);
        assert_eq!(m.retries_total, 2);
        assert_eq!(m.failures_total, 2);
    }

    #[tokio::test]
    async fn non_retryable_fails_fast() {
        let mock = MockProvider::new("mock");
        mock.plan(vec![Err(VlmError::Schema("bad answer".into()))]);
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let err = o
            .morning_diff(&[], &jpg(320, 240), &["line_1".to_string()], &floorplan())
            .await
            .unwrap_err();
        assert!(matches!(err, VlmError::Schema(_)));
        let m = o.metrics().snapshot();
        assert_eq!(m.calls_total, 1, "schema errors are not retried");
        assert_eq!(m.retries_total, 0);
    }

    #[tokio::test]
    async fn circuit_opens_after_failure_burst_blocks_new_calls() {
        let mock = MockProvider::new("mock");
        mock.plan(vec![
            Err(VlmError::Schema("f1".into())),
            Err(VlmError::Schema("f2".into())),
            Err(VlmError::Schema("f3".into())),
        ]);
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        for _ in 0..3 {
            let _ = o
                .morning_diff(&[], &jpg(320, 240), &["line_1".to_string()], &floorplan())
                .await;
        }
        let sealed = o
            .morning_diff(&[], &jpg(320, 240), &["line_1".to_string()], &floorplan())
            .await;
        assert!(matches!(sealed, Err(VlmError::CircuitOpen { .. })));
        assert_eq!(
            o.metrics().snapshot().calls_total,
            3,
            "gated call never dialed"
        );
    }

    #[tokio::test]
    async fn oversized_crop_rejected_before_any_provider_call() {
        let mock = MockProvider::new("mock");
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let blocks = vec![MorningBlock {
            block_id: "big".into(),
            ssim_delta: -0.1,
            bbox_2d: [0.0; 4],
            crop_b64: jpg(1000, 100),
        }];
        let err = o
            .morning_diff(
                &blocks,
                &jpg(320, 240),
                &["line_1".to_string()],
                &floorplan(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, VlmError::InvalidPayload(m) if m.contains("side cap 640")));
    }

    #[tokio::test]
    async fn eod_clean_day_skips_cloud_call_and_reports_none() {
        let mock = MockProvider::new("mock");
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let out = o
            .eod_root_cause(&[], 100, 98, &[])
            .await
            .unwrap_or_else(|e| panic!("clean day: {e}"));
        assert!(out.is_none());
        assert_eq!(o.metrics().snapshot().calls_total, 0);
    }

    #[tokio::test]
    async fn eod_with_episodes_prompts_and_returns_report() {
        let answer = serde_json::json!({
            "culprit_window": {"ts_start": "2026-04-07T14:01:00Z", "ts_end": "2026-04-07T14:02:00Z"},
            "root_cause": "pallets misclassified as boxes",
            "corrected_boxes": [],
            "notes": ""
        });
        let mock = MockProvider::new("mock");
        mock.respond_with("root_cause_localization", answer);
        let o = orch(mock, DEFAULT_DAILY_USD_CAP);
        let epoch = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap_or(Utc::now());
        let window = crate::root_cause::MinuteWindow {
            start: epoch,
            end: epoch + std::time::Duration::from_secs(60),
            max_uncertainty: 1.0,
            total_divergence: 0.0,
        };
        let ep = CulpritEpisode {
            window,
            frames: vec![crate::root_cause::FrameSample {
                ts: epoch + std::time::Duration::from_secs(10),
                frame_b64: jpg(320, 240),
                pre_boxes: vec![[0.0; 4]],
            }],
        };
        let report = o
            .eod_root_cause(&[ep], 1485, 1420, &["PALLET_AS_BOX".into()])
            .await
            .unwrap_or_else(|e| panic!("eod: {e}"))
            .unwrap_or_else(|| panic!("expected report"));
        assert_eq!(report.root_cause, "pallets misclassified as boxes");
    }

    #[tokio::test]
    async fn coco_collection_gates_via_acceptance_rules() {
        let o = orch(MockProvider::new("mock"), DEFAULT_DAILY_USD_CAP);
        assert_eq!(o.label_decision(0.97, true, 3), LabelDecision::AutoAccept);
        assert_eq!(o.label_decision(0.97, false, 3), LabelDecision::HoldReview);
        let items = vec![crate::auto_label::LabelItem {
            file_name: "a.jpg".into(),
            height: 64,
            width: 64,
            bbox: [1.0; 4],
            category: "pallet".into(),
            confidence: 0.97,
            from_known_root: true,
        }];
        let coco = o.collect_coco(&items, &["pallet".to_string()]);
        assert_eq!(coco.images.len(), 1);
    }

    #[tokio::test]
    async fn over_cap_day_blocks_auto_label_via_preflight_only() {
        // execute()? is private; preflight is the enforcement point — verify
        // through the guardrails surface here for regression pinning.
        let mut budget = DailyBudget::new(0.01);
        let now = Utc::now();
        budget.charge(10.0, now);
        assert!(crate::guardrails::preflight(OpKind::AutoLabel, &budget, now).is_err());
        assert!(crate::guardrails::preflight(OpKind::MorningDiff, &budget, now).is_ok());
    }
}
