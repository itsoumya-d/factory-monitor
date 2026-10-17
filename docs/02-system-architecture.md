# 02 — System Architecture

## 1. Hard Partitioning Rule

The system is split into two runtime worlds with a **one-way clock-speed contract**:

```
        ULTRA-FAST WORLD (per-frame, must never block)            ASYNC SLOW WORLD
┌────────────────────────────────────────────────┐   ┌─────────────────────────────────────────┐
│  ingestion-cctv ──frame──▶ ingestion-wifi ──── │   │  api-server (axum REST/WS, webhooks)    │
│        │                       │               │   │  vlm-orchestrator (cloud calls)          │
│        └───▶ inference-engine ─┴──▶ fusion     │──▶│  rl-controller (policy update, EOD)     │
│             (ort: detect → ByteTrack → count)  │   │  TimescaleDB / Qdrant                    │
└────────────────────────────────────────────────┘   └─────────────────────────────────────────┘
        lock-free SPSC/MPSC rings, no allocations                tokio tasks, channels, reqwest
        in hot loop, no .await, no DB
```

**Contract:** nothing in the fast world ever does `.await`, allocations, DB I/O, logging at `info+`, or channel send that could block (`try_send` only, drop-on-full with a drop counter metric capped at 0.1% of frames).

## 2. High-Level Data Flow

```
[RTSP cams H.264/265] ──GStreamer/NVDEC──▶ FrameRing ──┐
[ESP32-S3 / AX210 CSI] ──UDP/serial──▶ CsiWindow ──────┤
                                                       ▼
                              ┌──── CrossModalAligner (100ms sliding window) ────┐
                              │                                                  │
                              ▼                                                  ▼
                   inference-engine                                  wifi-model (ort: CSI 1D-CNN+TFM)
                   • ort session (RF-DETR/YOLO26)                    • zone occupancy
                   • ByteTrack (Kalman+Hungarian)                    • motion cadence p(W)
                   • ROI line-crossing counter                       • pallet density estimate
                              │                                                  │
                              └────────▶ ModalityFusionEngine ◀───────────────────┘
                                            │
                              PackageCountEvent{count, conf, uncertainty, modality_blend}
                                            │
                        ┌───────────────────┼────────────────────┐
                        ▼                   ▼                    ▼
                 LiveDashboard(WS)   TimescaleDB(counts/   HardNegativeBuffer
                                     telemetry/RL log)       (frames+CSI ambig.)
                                            │
                                            ▼ (EOD)
                          rl-controller: reward = -|N_pred - N_actual| - λ·FP
                          → clamp-bounded threshold/weight update (fast loop)
                          → suspect-window frames → VLM auto-label (slow loop)
                          → nightly LoRA fine-tune → ONNX hot-swap @ next boot
```

## 3. Crate Map & Responsibilities

| Crate | Owns | Never does |
|---|---|---|
| `common-types` | `FactoryEnvState`, `PackageCountEvent`, `HumanAuditFeedback`, `RlAction`, traits `CsiSource`, `VisionDetector`, `PolicyAgent` | I/O of any kind |
| `ingestion-cctv` | RTSP clients, GStreamer pipelines, luminance trigger, golden-baseline SSIM, frame ring buffer | model inference |
| `ingestion-wifi` | CSI packet parsers (ESP32-S3 UDP/serial, AX210 pcap), phase unwrap/sanitization, FFT, sliding windows | vision |
| `inference-engine` | `ort` session pool, detector wrappers, ByteTrack, line-crossing math, fusion engine | network, DB |
| `rl-controller` | bandit/PPO policy net (`burn`), reward calc, guardrail clamping, threshold bus | real-time frame work |
| `vlm-orchestrator` | OpenAI/Gemini-compatible client, morning diffing, EOD root-cause payload, COCO auto-label export | any raw frame processing beyond encode/send |
| `api-server` | axum REST (+ tonic gRPC optional), WS fan-out, webhook dispatcher, sqlx persistence, config | inference math |

## 4. Threading & Runtime Model

- **Main runtime:** one `tokio` multi-thread runtime (workers = physical cores).
- **Fast paths** run on dedicated OS threads spawned outside tokio:
  - `cctv_decode_n` threads (one per camera NVDEC pipeline),
  - `infer_thread` (owns the `ort` session; messages passed via `ArrayQueue`/`crossbeam-channel`),
  - `wifi_proc` thread pool (2 threads, FFT + sanitize).
- **Rationale:** `ort` CUDA sessions are not `Send`-safe across arbitrary awaits; keeping one pinned thread per GPU session avoids sync overhead and keeps GC-pausal/no-drop latencies deterministic.
- **Channels:**
  - `frames → aligner → infer`: `rtrb`/`crossbeam` ArrayQueue, capacity = 4× stream rate; on overflow **drop-oldest** with a monotonic `dropped_frames` counter.
  - `events → api-server`: `tokio::sync::broadcast` (dashboard fan-out) + `tokio::sync::mpsc` (DB persist).

## 5. Failure Isolation

| Failure | Isolation behavior |
|---|---|
| One RTSP camera stalls | per-camera supervised task; decoder watchdog emits `StreamDegraded{camera_id}`; fusion keeps CSI-only mode for that zone |
| GPU/CUDA died | `ort` switch to CPU EP with degraded FPS; alert webhook; no crash |
| VLM API outage | morning boot degrades to SSIM-only diff (flags CHANGED, skips AI classification); EOD correction still accepted, labeling queued |
| CSI source offline | vision-only counting; uncertainty flag forced ON; PoE switch watchdog ping |
| Operator submits garbage correction | schema validation + plausibility bounds (negative counts, >5× swing requires confirm dialog) |
| RL update rejected (would clampblind) | watchdog reverts to last-safe action set + alert (see docs/08 §6) |

## 6. Deployment Topology

```
        ┌──────────────── Factory Edge Server (GPU) ───────────────┐
        │  Rust Engine (all 7 crates)   TimescaleDB   Qdrant        │
        │  TimescaleDB runs in Docker; raw frame ring on NVMe tmpfs  │
        └────────────┬───────────────────────┬─────────────────────┘
                     │ RTSP (local VLAN)     │ UDP CSI :9000 / serial
             [4–12 IP cams]            [2–4 ESP32-S3 CSI nodes]
                                             + local NTP/PTP time server
                     │ HTTPS/VLM only outbound, via firewall allow-list
                     ▼
              [VLM API]  [ERP/MES webhooks]
```

- Everything video + CSI stays **inside** the factory LAN. Only: VLM calls (images of ambiguous frames), webhooks, WS to dashboard may leave — each behind an explicit allow-list & secret store.

## 7. State Machine (system lifecycle)

```
Standby ──(lights-on AND csi_stable ≥ 30s)──▶ Calibrating
Calibrating ──(SSIM OK ∧ VLM OK ∧ ROIs confirmed)──▶ Counting
Calibrating ──(layout change detected)──▶ Calibrating{ROI_redraw} → Counting
Counting ──(luminance < threshold for 5min)──▶ Standby
Counting ──(EOD correction posted)──▶ Correcting → Counting
Any ──(watchdog fault ×3)──▶ Degraded →(manual ack)→ previous state
```

Transitions are persisted (`calibration_state` table) and idempotent on restart.
