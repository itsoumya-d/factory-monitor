# 17 — Universal Multimodal Spatial Intelligence Platform (Layer Charter)

Status: **charter** — the master prompt's five-layer platform crossed with the
docs/16 route. docs/16 is unchanged; this doc widens it again. The factory
product docs/01–15 stays instance #1; every new layer is industry-agnostic.

---

## 1. Layer crates and what they own

| Layer | Crate | Owns | CI compile mode |
|---|---|---|---|
| A | `crates/esp-csi-node` | ESP32-C5/C6/S3 CSI firmware components: 4-byte LTF strip parser, amplitude/RP/BVP+Doppler preprocessing, radix-2 FFT (core-only), capability guardrails, the 5 anti-shortcut diagnostic controls, 30 s empty-room + door-swing negative controls, esp-crab dual-chip phase-sync planner | `#![cfg_attr(not(test), no_std)]`; vendor deps target-gated (xtensa / riscv32), so `cargo test --workspace` runs the DSP on host |
| B | `crates/edge-core-rs` | Cross-modal fusion transformer seam (RGB/depth/CSI tokens), FIA dual-stream occlusion recovery (FSM kernels 1×3×3 | 3×1×1 | 1×1×1 | 5×1×1, NTA), Speed-1 prompt patch + ≤2 s replay verifier, Speed-2 clamped linear bandit (τ ∈ [0.25, 0.85], α/β weights), Speed-3 regression gate | std; `ort`/TensorRT behind `tensorrt` feature, default mock seams |
| C | `crates/universal-bridge-rs` | `oxvif`-style ONVIF deep verifier + camera clone mock transcript, MoQT pub/sub seam + HS256 `vts` JWTs + cached FETCH rewind, `RegisterRepresentable` f32/f64 → u16 registers across abcd/badc/cdab/dcba, OPC UA tag plan, rig-core 0.44 + mcp-core tool contracts, TPM 2.0 NV-key signer + C2PA fragments | std; vendor deps (tokio-modbus 0.17, opcua, rig-core, mcp-core, tss-esapi, c2pa) each behind an optional feature so hermetic CI never resolves them |
| D | `crates/compliance-guard` | EU AI Act Art 5(1)(f) emotion-lock validator (compile-adjacent + runtime), Annex III safeguards (station/zone aggregation, 1:N biometrics block, volatile-RGB policy), subject-level ε-DP clipping + Gaussian noise | full std, no vendor deps |
| E | `crates/spatial-studio-ui` | 4-stage 15-min onboarding state machine, 90-day ROI ledger, EU AI Act scorecard calc | std; `dioxus` + `wasm-bindgen` gated on `target_family = "wasm"` |
| shared | `crates/monitoring-contract` | The universal typed rule schema (docs/16 §2) — contract, targets, geometry, rules, CSI rules, triggers, webhooks, KPIs, vertical templates (manufacturing / QSR / retail-warehouse / healthcare) | std |

Why the extra two crates beyond the master prompt's five: the charter of
record (docs/16) already routes the rule schema to `monitoring-contract` and
the node self-test DAG to `node-diagnostics`; thinning these out of the five
layers keeps the mandated crates focused. `DiagnosableNode` trait + node
health/self-test types land in this doc's D2 scope and the control *executors*
in `esp-csi-node::diag`.

## 2. Deliverable order (master prompt, mapped)

1. **D1 scaffold** — workspace members + feature flags; every crate compiles
   and tests hermetically (no vendor deps resolved in CI).
2. **D2 core types** — `monitoring-contract` full schema + validator + 4
   vertical templates; `compliance-guard` Art 5 validator + Annex III +
   ε-DP; `DiagnosableNode` trait + the five anti-shortcut controls.
3. **D3 perception** — `esp-csi-node` parser/DSP/guardrails/audits + phase-sync
   planner; `edge-core-rs` fusion/FIA seams + three-speed controller.
4. **D4 bridge** — deep camera verifier + clone transcript, MoQT seam + JWT,
   `RegisterRepresentable` endian codec, OPC UA plan, agent tool contracts,
   TPM + C2PA signer.
5. **D5 UI** — onboarding state machine, ROI ledger, WASM shell (wasm target
   only).

## 3. Invariants carried over from docs/16

- counts pipeline budget (≤ 40 ms) may not regress; fusion/FIA work is
  event-driven tiers, never inside ByteTrack's hot loop via mocks.
- RL touches only thresholds + fusion weights; guardrail clamps hold
  (`τ_conf ∈ [0.25, 0.85]`, α/β ∈ [0,1] normalized) — enforced in `Speed-2`.
- No `unwrap`/`expect`/`panic!` in prod code (gate script), two-branch
  workflow (`test-debug` → `main` only after fmt+clippy+tests+CI green).
- Emotion/mood/sentiment/honesty inference is globally denied
  (Art 5(1)(f)); allowed human states are the medical/safety whitelist only.
- All statistical aggregation is station/zone keyed; raw RGB dies in RAM;
  Σε budget per subject is tracked, ε ∈ [1, 200] admin-set.

## 4. Vendor dependency posture

Every vendor crate named in the master prompt is declared per-crate as
`optional = true` / target-gated and enabled by a named feature:

- `esp-csi-node`: `esp-csi-rs` 0.13, `esp-hal` ~1.2, `embassy-executor` 0.10,
  `defmt` 1.0 — gated on `xtensa` / `riscv32` targets.
- `edge-core-rs`: `ort` (TensorRT EP) behind `tensorrt`.
- `universal-bridge-rs`: `tokio-modbus` 0.17 (`modbus`), `opcua` (`opcua`),
  `rig-core` 0.44 + `mcp-core` (`agent`), `tss-esapi` (`tpm2`), `c2pa`
  (`c2pa`), `moqt` (`moqt`), `oxvif`-client behavior behind `oxvif`.
- `spatial-studio-ui`: `dioxus` (`web` features) gated on wasm target.

Hermetic default = CI stays green regardless of vendor versions; the feature
builds are validated on device/pilot hardware, not in CI.
