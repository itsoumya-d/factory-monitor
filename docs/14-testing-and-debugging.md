# 14 — Testing & Debugging Strategy (the `test-debug` branch blueprint)

This document is owned by the **`test-debug` branch**. Nothing merges to `main` until every applicable gate below is green. Testing philosophy: **synthetic-first, deterministic, replayable, hardware-last.** Every test must be runnable on a laptop without GPU.

## 1. The four test layers

| Layer | Tool | Scope | Gate |
|---|---|---|---|
| L1 Unit | `cargo test`, `proptest` | pure functions: bounds clamping, SSIM math, phase unwrap, Hungarian, reward calc | 100% pass + ≥ 85% line coverage on `common-types`, `rl-controller` |
| L2 Integration | `cargo test --features itest` | crate-to-crate: fake sources → fusion → fake DB (sqlx test harness, `#[sqlx::test]`) | all green, no `.unwrap` clippy warnings (`#![deny(clippy::unwrap_used)]` in lib crates) |
| L3 System simulation | `cargo run -p sim-replay` + Playwright | whole engine on fixture day: 8 h synthetic camera day + CSI synthetic walk; assert day totals exact | scenario pass matrix (below) |
| L4 Hardware soak ("burn-in kit") | bench rig, 2 cams + 2 ESP32-S3 | 7-day continuous run with scripted occlusion/lighting events | zero crashes, count error ≤ 3%, clock alarms logged & resolved |

## 2. Determinism rules (every test obeys)

- Time: inject `Clock` trait (mock-instant) — no `Utc::now()` in unit paths; real time only at binary edge & PTP shim behind trait.
- Sources: `SourceAdapter` fake with fixed seeds; `ReplayCsi` + file-backed RTSP/H.264 fixtures.
- Vision: model is **mocked with a scripted detector** (`MockDetector` returns hand-authored boxes per fixture frame) — real ONNX tests run separately (`heavy` feature) on GPU CI.
- Reward math: golden tests with `insta` snapshots.

## 3. Fixture corpus (committed under `fixtures/`)

| Fixture | Description | Used by |
|---|---|---|
| `day_normal.json` | 8 h synthetic: 4 zones, predictable crossings, no occlusion | L3 |
| `day_occlusion.json` | scripted forklift occlusion 14:00–14:40 + Wi-Fi motion continuity | fusion tests |
| `day_lighting.json` | dawn ramp, noon glare band, night floodlight | luminance trigger + RL state tests |
| `csi_walk.npy`, `csi_idle.npy`, `csi_forklift.npy` | real captured ESP32-S3 matrices (sanitized) | CSI pipeline golden tests |
| `frames_line2_seq.h264` | 30 s real H.264 excerpt from partner line | GStreamer path (feature-gated on CI GPU or ffmpeg) |
| `corrections_sample.jsonl` | 20 realistic EOD HumanAuditFeedback payloads | RL + report tests |
| `vlm_responses/*.json` | canned VLM JSON outputs (diff, root cause, boxes) | vlm-orchestrator mock |

## 4. Per-module test requirements (highlight — the pyro traps)

### bounds & RL (docs/08)
-_property tests_: for ANY action vector, `RlAction::clamped` lands in every SAFE range; blend normalizes to 1.0.
- watchdog rejection: moving τ_conf farther than ±0.15 in one epoch → rejected; 3 rejects → freeze + restore last-safe.
- reward golden: `-|1485-1420| - λ·FP` snapshot tests for λ ∈ {0.5, 1.0, 3.0}.

### byteTrack + crossing
- `lapjv` vs brute force on ≤ 6×6 costs (golden eq tests).
- Track ID stability across 3-frame occlusion gap (buffer=45) vs perish when gap > buffer.
- Crossing direction: A→B counted, B→A discarded; double-cross jitter (±0.5 px) does NOT double count.
- Line-crossing math property: intersection symmetric, no dup events for same track+line within 10 frames (debounce).

### CSI
- Phase unwrap idempotent + monotonic-safe under synthetic wrap cases (2π crosses at edges).
- Outlier packet drop (inject 3.5σ packet) removes it without window corruption.
- FFT pipeline: csi_walk.npy → presence_prob > 0.8, csi_idle.npy → < 0.2 (golden).
- Packet-loss injected 5% / 20%: metrics report, no panic, gap interpolation as spec (docs/05 §6).

### fusion
- Divergence matrix truth table (vision ± wifi deltas {0,1,2,3,5} × occlusion ratio) → expected flags + counts (docs/07 §5).
- Clock degraded: force `clock_ok=false` → `CLOCK_DEGRADED` flag mandatory on every event, tolerance widens.

### api-server
- `#[sqlx::test]` for repos; outbox retry semantics (4xx → dead, 5xx → backoff).
- WS: 200 clients fan-out synthetic day, no dropped events (idempotency resync).
- HMAC signature verification positive/negative.

### dashboard (Playwright)
- Correction form: normal delta submits; >25% requires confirm; retrain checkbox propagates.
- WS replay `day_normal.json` → counters match fixture ground truth exactly.

## 5. Cross-modal "burn-down" day test (the money test)

`sim-replay day_occlusion.json`: end of day engine must report total = fixture truth 1,420 (±0 in synthetic mode), with:
- occluded spans flagged `OCCLUDED_ESTIMATE` exactly on fixture timestamps,
- correction of +38 (fixture embeds an operator correction) → reward computed to golden snapshot,
- nightly: `nightly_fine_tune --dry-run` produces a COCO export matching fixture labels (no network).

## 6. Static gates (CI, enforced)

```
cargo clippy --workspace --all-targets -- -D warnings -W clippy::unwrap_used
cargo fmt --check
cargo deny check licenses advisories
cargo test --workspace
wasm-pack build dashboard (no — dashboard is node: pnpm lint && pnpm tsc --noEmit)
```

## 7. Performance benches (criterion) — fail if budget × 1.05 exceeded

From docs/07 §7 budgets. Benches run on CI GPU runner; laptop runs CPU-only variant with relaxed budgets (documented).

Boxed mounts:
- `bench_ingest_decode_frames_per_s`
- `bench_detect_640_ms`
- `bench_bytetrack_300_dets_us`
- `bench_fusion_tick_us`
- `bench_csi_fft_window_ms`

## 8. Debug tooling (for humans)

- `RUST_LOG=debug shado-engine replay fixtures/day_occlusion.json --speed 4x --tap out/`
- Taps: raw pre/post preprocess PNG dumps, detection JSONL per frame, CSI matrices as npy, per-tick fusion decisions — all under `--tap` only (never in prod).
- `/api/v1/admin/models/selftest` — run detector golden-check on 5 canonical frames, report per-class score.
- `shado-doctor` binary: env sanity (GPU, EPs, NTP offset, DB migrations, camera reachability) for support.

## 9. Test↔debug operating flow on the branch

1. Write failing test first for the bug/feature.
2. Reproduce red locally with taps to inspect pipeline state.
3. Fix; gates L1→L3 pass locally; benches if perf-path code touched.
4. Commit on `test-debug` with scope tag (`fix(rl): clamp reject frame cause`). CI runs full matrix.
5. For soak-worthy changes: annotate a "burn-in needed" label → runs on L4 bench rig 24 h with synthetic event scripts.
6. When Definition of Done met (docs/15 §5) → merge commit to `main` with `MERGED: <summary of what was tested>` footer listing gate results.
