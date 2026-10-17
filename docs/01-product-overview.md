# 01 — Product Overview

## 1. Problem Statement

Factories need accurate counts of completed packages/units and worker activity per shift for payroll, ERP/MES integration, and throughput planning. Camera-only approaches fail each day to:

- **Occlusion:** forklifts, stacked pallets, and human bodies block the belt.
- **Lighting shifts:** dawn/midday/night produce glare, shadow bands, and color casts.
- **Environmental noise:** steam, dust, and seasonal fog (invisible: burned/infrared vision fails).
- **Layout drift:** machinery, pallet jacks, and staging tables move between lifecycles.
- **Class drift:** box/shape/pallet color changes (new SKUs) cause silent miscounts.

Plain (optical) counting silently fails exactly when it matters — at end of day, the discrepancy is discovered and there is no recovery path.

## 2. Solution

Dense-shape **multimodal sensing**:

1. **CCTV (video)** — primary modality: object detection (RF-DETR/YOLO26 in ONNX via `ort`), ByteTrack tracking, virtual line-crossing counts.
2. **Wi-Fi CSI (radio)** — through-occlusion presence sensing: 56–192 OFDM subcarriers bend/scatter around bodies and packages; a 1D-CNN + Transformer classifier in Rust outputs zone occupancy, motion cadence, and motion energy even when monitors/screen light changes.
3. **Sensor fusion engine** — cross-validates visual counts with CSI motion spikes; produces a unified `PackageCountEvent { count, confidence, uncertainty_flag, modality_blend }`.
4. **RL self-correction loop** — a contextual-bandit/PPO policy adjusts detector thresholds + fusion weights during the day (fast loop), guards against runaway drift by hard-clamped bounds (mechanical lines of safety).
5. **Human-in-the-loop EOD audit** — operators correct daily totals via the dashboard; corrections are (a) consumed as RL reward/penalty, (b) localized to culprit timestamp windows, (c) shipped to a VLM for auto-labeling during a nightly active-learning fine-tune.

## 3. Users & Personas

| Persona | Needs | Interface |
|---|---|---|
| **Plant Manager** | Accurate EOD totals, bottleneck heatmaps, shift reports, Root-cause vs. trust during change | Dashboard `Audit` tab + `Review` tab |
| **Line Operator** | Someone to tell the system when it's wrong (fast, no training) | One-click EOD correction form |
| **Maintainability Engineer** | "Fix the count without re-uploading ~3 weeks of data" | Alerts + Slack/Teams + auto-generated tuning notes |
| **DevOps (SRE)** | One-command deploy, crash-safe restarts, observability | `docker compose`, Prometheus/Grafana, structured logging via `tracing` |
| **ML Engineer** | Iterate on model, monitor drift, roll back | `docs/14-testing-and-debugging.md`, model registry, CI |

## 4. Success Metrics (Definition of "Working")

| Metric | Target |
|---|---|
| Package count accuracy (normal day) | ≥ 98.5% |
| Package count accuracy (occluded zone) | ≥ 95% |
| End-to-end detection latency budget (camera → event) | ≤ 250 ms p95 |
| Edge inference throughput | ≥ 2 streams × 30 FPS on NVDEC + TensorRT |
| Auto-recovery after power loss | < 90 s, no data corruption |
| EOD correction absorption → behavior change | policy threshold shift visible next morning; retrained weights hot-swap without downtime |
| False-"blind" incidents from RL (detectors disabled) | 0 (guardrails) |

## 5. Out of Scope (v1)

- Selling to untrained factories without a plant manager; the loop depends on a human-in-the-loop at EOD audit.
- Tracking individual workers' personal identity/biometrics. We count packages/tasks and aggregate motion — no face recognition, no personal metadata storage.
- Fully cloud-hosted inference (edge-first: VLM + fine-tune are the only cloud calls).
- Fully autonomous "shop floor supervisor" agents — the RL agent owns **thresholds & fusion weights only**, never counts directly, never writes to the ERP.

## 6. Key Risk Register

| # | Risk | Mitigation |
|---|---|---|
| R1 | CSI hardware variance (ESP32-S3 vs AX210) | Abstract `CsiSource` provider trait; per-device calibration profiles (see `docs/05-wifi-sensing.md`) |
| R2 | RL runaway (threshold → 0 or 1 blinds detector) | Hard-clamped action space + recovery watchdog (see `docs/08-rl-controller.md`) |
| R3 | Wi-Fi/CCTV timestamp skew | Edge-stamped NTP/PTP, 100 ms sliding fusion window | 
| R4 | VLM hallucinated labels poison training | Human review gates before nightly fine-tune; auto-labels stored in review-pending state |
| R5 | RTSP camera firmware quirk (keyframe cadence changes) | GStreamer decoder watchdog + automatic reconnect w/ exponential backoff |
| R6 | Catastrophic forgetting in overnight fine-tune | LoRA-only freeze + replay buffer of prior hard negatives + morning A/B shadow eval |
| R7 | TimescaleDB disk growth | Ring-buffer-first design for raw frames/CSI (RAM/disk evict), compress hyper-table chunks | 
