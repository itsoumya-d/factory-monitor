# Shado — Self-Correcting Multimodal Factory Monitoring Engine

A production-grade system that monitors a factory floor using synchronized **RTSP CCTV video** and **Wi-Fi Channel State Information (CSI)** radio signals, counts completed packages/worker work through **multimodal sensor fusion**, and self-corrects via a **human-in-the-loop Reinforcement Learning (RL) + active learning** feedback loop that fine-tunes the vision model overnight.

> **Status:** Documentation Phase (v0.1). Code skeletons land on the `test-debug` branch only after passing the test & debug gates defined in `docs/14-testing-and-debugging.md`, then merge to `main`.

---

## 1. What This System Does (Daily Lifecycle)

| Phase | Name | Summary |
|-------|------|---------|
| 1 | **Morning Boot & Calibration** | Detects lights-on (frame luminance trigger) + Wi-Fi CSI stabilization → captures reference snapshots → VLM layout diffing vs. yesterday's golden baseline → fires webhook calls to ERP/MES/alerting → recalibrates virtual counting lines (ROIs). |
| 2 | **Intraday Multimodal Counting** | ONNX detector (RF-DETR / YOLO26) → ByteTrack persistent IDs → virtual line-crossing counts. Wi-Fi CSI (rustfft + phase sanitization) cross-verifies count during occlusion, glare, steam, dust. Fused `PackageCountEvent` with confidence + uncertainty flags. |
| 3 | **EOD Human Feedback & RL Loop** | Visual business report (hourly throughput, bottleneck heatmaps, low-confidence windows). Operator submits corrections ("Line 4 counted 1,485, actually 1,420 — empty pallets counted as boxes PM shift"). System: computes RL reward penalty → adjusts thresholds/fusion weights within guardrail bounds → sends suspect frames to VLM for auto-labeling → queues nightly LoRA/ONNX fine-tune → hot-swaps weights at next morning boot. |

## 2. Repo Structure (target state)

```
factory-monitor/
├── README.md                        # this file
├── docs/                            # ← full build documentation (the single source of truth)
│   ├── 01-product-overview.md
│   ├── 02-system-architecture.md
│   ├── 03-technology-stack.md
│   ├── 04-cctv-ingestion.md
│   ├── 05-wifi-sensing.md
│   ├── 06-core-data-structures.md
│   ├── 07-inference-engine.md
│   ├── 08-rl-controller.md
│   ├── 09-vlm-orchestrator.md
│   ├── 10-api-server.md
│   ├── 11-database-schema.md
│   ├── 12-dashboard-spec.md
│   ├── 13-deployment-ops.md
│   ├── 14-testing-and-debugging.md
│   └── 15-build-workflow.md
├── crates/                          # Rust workspace (phase 2, on test-debug branch)
│   ├── common-types/                # shared contracts: FactoryEnvState, PackageCountEvent, traits
│   ├── ingestion-cctv/              # RTSP decode, luminance trigger, ring buffer
│   ├── ingestion-wifi/              # CSI parser, phase unwrap, FFT features
│   ├── inference-engine/            # ort sessions, detectors, ByteTrack, fusion
│   ├── rl-controller/               # contextual bandit/PPO policy, reward calc
│   ├── vlm-orchestrator/            # VLM client: scene diff, root-cause, auto-label
│   └── api-server/                  # axum REST + WebSocket, webhooks, persistence
├── dashboard/                       # Next.js visual business platform (phase 3)
└── migrations/                      # Postgres+TimescaleDB SQL migrations
```

## 3. Branching Model (strict)

- **`main`** — production-ready only. Everything here has been tested & debugged first. Docs land here once complete; code lands here only via merge from the test branch after gates pass.
- **`test-debug`** — the working branch. All implementation, unit/integration/bench tests, debugging, and CI runs happen here. When the Definition of Done (`docs/15-build-workflow.md` §5) is met, it merges into `main` with a fast-forward/PR flow.

```
main      ──●────────────────────●────────────────
             \                    ↑ (merge after all tests/debug gates pass)
              `──●── test/debug work ●──●──● ──
                 (test-debug)
```

## 4. Quick Doc Navigation

| I want to… | Read |
|---|---|
| Understand the product & users | `docs/01-product-overview.md` |
| See how crates fit together | `docs/02-system-architecture.md` |
| Check crate/pin choices | `docs/03-technology-stack.md` |
| Build the CCTV pipeline | `docs/04-cctv-ingestion.md` |
| Build the Wi-Fi CSI pipeline | `docs/05-wifi-sensing.md` |
| Implement core types/traits | `docs/06-core-data-structures.md` |
| Implement detection/tracking/fusion | `docs/07-inference-engine.md` |
| Implement the RL loop safely | `docs/08-rl-controller.md` |
| Integrate VLM APIs | `docs/09-vlm-orchestrator.md` |
| Build the axum API | `docs/10-api-server.md` |
| Create DB migrations | `docs/11-database-schema.md` |
| Build the Next.js dashboard | `docs/12-dashboard-spec.md` |
| Deploy hardware & ops | `docs/13-deployment-ops.md` |
| Test & debug | `docs/14-testing-and-debugging.md` |
| Follow build/merge workflow | `docs/15-build-workflow.md` |

## 5. Non-Negotiable Engineering Rules

1. **No `.unwrap()`/`expect()` in production paths.** `thiserror` for library crates, `anyhow` at binary edges.
2. **RL guardrails are absolute:** `τ_conf ∈ [0.25, 0.85]`, `τ_iou ∈ [0.30, 0.70]`, fusion weights `α_vision, β_wifi ∈ [0.05, 0.95]` with `α + β = 1`. A bad human report can never blind the detector (see `docs/08-rl-controller.md` §4).
3. **Time synchronization is mandatory:** CCTV frames and CSI bursts are edge-stamped via local NTP/PTP, aligned in a 100 ms sliding window before fusion (see `docs/13-deployment-ops.md` §3).
4. **Every automated count is auditable:** predictions, sensor states, thresholds used, and model version are persisted per event (see `docs/11-database-schema.md`).
5. **VLM is a fallback + teacher, not the primary counter:** real-time counting is 100% local edge inference; cloud VLM handles ambiguous frames, morning diffing, EOD root-cause, and auto-labeling.

## 6. License & Node Requirements (hardware)

- Rust 2024 edition, `rust-toolchain.toml` pins stable.
- Wi-Fi sensing nodes: **ESP32-S3** running esp-csi firmware (serial/UDP → Rust server) or Intel **AX210/5300** NICs on mini-PCs.
- Edge GPU: NVIDIA with NVDEC (GStreamer) + TensorRT/CUDA execution providers for `ort`.
