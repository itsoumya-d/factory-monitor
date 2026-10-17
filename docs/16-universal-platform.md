# 16 — Universal Prompt-Driven Platform (Charter of Record)

Status: **charter** — this doc reconciles the "universal, chat-driven monitoring
platform" vision with the existing docs/01–15 factory product. Nothing in this
charter deletes prior scope; it **widens** it. The factory deployment is
instance #1 of a class.

---

## 1. Reconciliation map

Existing crates keep their names and roles. New capabilities are **new crates**
plus surgical extensions — never rewrites of green crates.

| Universal concept (master prompt)                 | Shado home                                              | Action |
|---------------------------------------------------|---------------------------------------------------------|--------|
| `MonitoringContract` (typed rule graph)           | **new** `crates/monitoring-contract`                    | build (Phase 9) |
| Conversational onboarding + prompt compiler        | **new** `crates/onboarding-agent` (rig-core)            | build (Phase 10) |
| `DiagnosableNode` + self-test orchestrator + HITL  | **new** `crates/node-diagnostics`                       | build (Phase 11) |
| Two-tier open-vocabulary vision                    | extend `inference-engine` with `OpenVocabDetector` impl of existing `VisionDetector` trait | build (Phase 12) |
| `rl-self-healer` (three-speed recovery)            | extend planned `rl-controller` (Phase 4): same bandit core + **prompt-patch** and **nightly-retrain** hooks | build (Phase 4 + Phase 13) |
| Universal web dashboard & chat UI (Next.js)        | `dashboard` (docs/12) + chat surface; KPIs read from contract | Phase 14 |
| Wi-Fi CSI / RTSP ingestion, ByteTrack, fusion, DB   | unchanged (docs 04/05/07/11)                            | already green |

Invariants that may not regress: counts/thread budget (≤40 ms), RL touch only
thresholds+weights, no unwrap/panic in prod, CI gates on every merge.

## 2. The `MonitoringContract` (strongly typed rule graph)

An `MonitoringContract` version-N TOML/JSON document, produced by the
onboarding agent, executed by the engine. Shape (Rust, sketch):

```rust
pub struct MonitoringContract {
    pub contract_id: ContractId,       // ULID; semver'd per domain
    pub domain: DomainOntology,        // industry + vocabulary of work stages
    pub targets: Vec<OpenVocabTarget>, // visual prompts, negatives, tiers
    pub geometry: Vec<ContractZone>,   // zones → cameras, ROIs, tripwires, masks
    pub rules: Vec<MonitoringRule>,    // FSM rules: counts, dwells, states
    pub csi_rules: Vec<CsiRule>,       // per-zone RF co-verification
    pub triggers: Vec<StateTransitionTrigger>,
    pub webhooks: Vec<WebhookBinding>, // external API/ERP payloads
    pub kpis: Vec<KpiDefinition>,      // dashboard computes from event stream
}

pub struct OpenVocabTarget {
    pub class_id: u16,             // stable id used by tracker/rules
    pub display: String,           // admin-facing label
    pub tier1_prompts: Vec<String>,// hot-path text tokens (compiled CLIP)
    pub tier2_prompts: Vec<String>,// verification/referee tokens
    pub negatives: Vec<String>,    // e.g. "empty stacked cup", "steam cloud"
    pub min_score: f32,
}

pub enum MonitoringRule {
    CountLineCross { line_id, a_side, b_side, class_ids, debounce_frames, direction_whitelist },
    DwellTimer { polygon_id, enter_transition, warn_after, breach_after, class_ids },
    StateMachine { class_id, init, transitions },
    Presence { polygon_id, class_ids, at_least, during: PeriodSpec },
    CsiCorrelate { zone_id, metric, gate, must_agree /// none | warn | veto },
}

pub struct ContractZone { zone_id, camera_ids, polygons: Vec<Polygon>, lines: Vec<Line>, dwell_policies }
```

Contract → engine binding: `build_zone_rois(&ContractZone) ->
common_types::ZoneRois` (already-shipped line/mask math), FSM runtime per rule,
dwell timers as tick-side state. The counting pipeline's `CountableClass`
becomes `contract.class_ids` (still 0..N ints — ByteTrack unchanged).

## 3. Two-tier open-vocabulary pipeline (extends `inference-engine`)

```rust
pub trait TextPromptBackend: Send { async fn embed(&self, texts: &[String]) -> anyhow::Result<Array2<f32>>; }

pub struct OpenVocabDetector { /* ort session; TextPromptBackend; contract.view() */ }
// impl VisionDetector for OpenVocabDetector  ← existing trait, zero churn
```

- **Tier 1 (hot path ≤40 ms budget):** YOLO-World v2/YOLOE ONNX graph with
  per-class text-embedding inputs rebuilt only when the contract's tier-1
  prompts change (hot-reload, ms-scale).
- **Tier 2 (referee, event-triggered):** Grounding DINO 1.6 Edge or VLM API on
  motion/state-transition frames; adjudicates relational rules ("wiping, not
  walking past"). Tier-2 never blocks the 40 ms loop — it rewrites `score`s and
  emits `Tier2Verification` records into the same DB.
- Prompt normalization stays in the onboarding agent (§4), NOT in the hot path.

## 4. Onboarding agent (`rig-core`), conversational discovery

- Reconnaissance: boot-time snapshot of all camera streams + CSI multipath
  baselines (uses ingestion crates read-only, flags them as `Discovery` mode).
- Interview loop: LLM proposes environment summary ("commercial espresso
  bar..."), asks what "completed work" means, emits candidate
  `MonitoringContract` draft after each admin turn.
- Compiler role: admin text → `MonitoringContract` diff (prompts normalized to
  grounding tokens, geometry suggested from snapshots + admin confirmation).
- HITL canary (Phase 11 orchestrator) must approve a contract before it goes
  live; contracts are versioned and replayable.

## 5. Node diagnostics (`node-diagnostics`)

```rust
#[async_trait]
pub trait DiagnosableNode: Send {
    fn node_id(&self) -> NodeId;
    async fn health_check(&self) -> NodeHealthReport;
    async fn run_synthetic_self_test(&mut self) -> SelfTestResult;
    async fn human_verification_payload(&self) -> VerificationPayload; // HITL canary
}
```

Implementations wrap already-shipped pieces: `CctvNode` (frame rate, blur,
lux vs thresholds — consumes ingestion-cctv + preproc), `WifiCsiNode`
(packet rate, subcarrier SNR, baseline drift — ingestion-wifi), 
`OpenVocabVisionNode` (canary inference + annotated preview to admin chat),
`RlPolicyNode` (weights hash + action-space bounds check — rl-controller),
`WebhookDatabaseNode` (pg latency + external API probe). Morning suite =
orchestrated DAG; failure isolates node and tilts fusion weights to the
healthy modality (integration point with `ModalityBlend`).

## 6. Three-speed self-healing (extends rl-controller)

| Speed | Latency | Mechanism | Touchpoint |
|---|---|---|---|
| 1. Prompt & rule patch | <2 s | onboarding-agent rewrites contract negatives/rules; 5-min replay validation; admin confirm | monitoring-contract diff + inference-engine hot-reload |
| 2. RL adaptation | same shift | bandit/PPO acts (thresholds, α/β, dwell timers) | rl-controller Phase 4 |
| 3. Nightly distillation | overnight | ring negatives + VLM gold labels + ONNX fine-tune + regression gate; shadow promotion (<2% divergence, 2 h) | vlm-orchestrator (docs/09) + ring (docs/07 §6) |

Speed-1 changes are contract commits with audit trail; Speed-2 stays inside the
existing guardrail envelope; Speed-3 models always enter in shadow mode.

## 7. Cross-industry runbooks (contract examples, kept in repo)

`data/contracts/{cafe,manufacturing,warehouse,retail}/*.toml` — example
contracts used by integration tests so every phase is industry-agnostic by
construction. Tests compile each example and run the pipeline against it.

## 8. Extended build workflow (amends docs/15 phase table)

```
Phase 4  rl-controller bandit (+ ransom reward, watchdog, persistence)   ← current
Phase 5  vlm-orchestrator (morning diff, root-cause, COCO export)
Phase 6  api-server routes, WS, webhooks, persistence, boot order
Phase 7  dashboard live + audit + report (contract-adaptive KPIs)
Phase 8  real hardware soak (L4), fine-tune flag, hot-swap, guardrails
Phase 9  monitoring-contract crate + example contracts + contract→ROI bridge
Phase 10 onboarding-agent (rig-core discovery → contract compiler) + HITL chat handoff
Phase 11 node-diagnostics (DiagnosableNode, orch, canary approvals)
Phase 12 two-tier open-vocab vision in inference-engine (ort + TextPromptBackend)
Phase 13 three-speed self-healing integration (prompt patch w/ replay tests)
Phase 14 universal dashboard/chat (Next.js): chat, node matrix, contract-adaptive KPIs
```

Each phase: same gates (fmt/clippy −D unwrap, full tests, gitleaks/deny), same
`test-debug` → `main` flow.
