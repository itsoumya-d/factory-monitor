# 03 — Technology Stack (pinned)

## 1. Rust Workspace (edition 2024)

Root `Cargo.toml` pins:

```toml
[workspace.package]
edition = "2024"
rust-version = "1.85"

[workspace.dependencies]
tokio        = { version = "1",  features = ["full"] }
axum         = { version = "0.8", features = ["ws"] }
tower        = "0.5"
tower-http   = { version = "0.6", features = ["cors", "trace"] }
tonic        = "0.13"                # optional gRPC surface (ERP/MES integration)
reqwest      = { version = "0.12", features = ["json", "rustls-tls", "multipart", "stream"] }
sqlx         = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "chrono", "migrate"] }
serde        = { version = "1",  features = ["derive"] }
serde_json   = "1"
thiserror    = "2"
anyhow       = "1"
tracing      = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
chrono       = { version = "0.4", features = ["serde"] }
ndarray      = { version = "0.16", features = ["serde"] }
rustfft      = "6"
rustdct      = "0.7"
nalgebra     = { version = "0.33", features = ["serde-serialize"] }  # Kalman
ort          = { version = "2.0.0-rc", features = ["cuda", "tensorrt"] }
gstreamer    = "0.23"
gstreamer-app = "0.23"
gstreamer-video = "0.23"
opencv       = { version = "0.94", features = ["clang-runtime"] }    # optional fallback path
pcap         = "2"               # AX210 NIC CSI capture
tokio-serial  = "5"               # ESP32-S3 USB-serial source
qdrant-client = "1.13"
burn         = { version = "0.18", features = ["ndarray", "wgpu"] }  # RL policy training
lapjv        = "0.1"             # Hungarian assignment (Jonker–Volgenant)
image        = { version = "0.25", default-features = false, features = ["jpeg", "png"] }
base64       = "0.22"
prometheus   = { version = "0.14", features = ["process"] }
uuid         = { version = "1", features = ["v7", "serde"] }        # time-sortable ids
config       = { version = "0.15", features = ["toml", "environment"] }
rand         = "0.9"
proptest     = "1"
criterion    = { version = "0.5", features = ["html_reports"] }
insta        = { version = "1", features = ["yaml"] }
mock-instant = "0.3"
```

### Version decision notes

| Choice | Why this, over alternatives |
|---|---|
| `gstreamer-rs` primary, `opencv`/`ffmpeg-next` optional | GStreamer gives NVDEC hw decode + RTSP robustness (rtpjitterbuffer) + zero-copy frames. opencv is the test-friendly fallback for luminance-only paths on non-GPU CI. |
| `ort` 2.0-rc | Only maintained ONNX Runtime binding with TensorRT/CUDA EPs and dynamic InferSessionArchetype in Rust. `burn` does weight training in-memory (RL), `ort` owns runtime inference. |
| `burn` for RL, not `candle` | burn has stable optimizer/backprop + wgpu CPU/GPU backends both usable for tiny policy nets; candle's trainer API churns faster. candle is allowed in-vision for experiments only. |
| `nalgebra` + `lapjv` | ByteTrack needs: Kalman filter (nalgebra) + Hungarian cost solve (lapjv). Implementing our own JV solver is两周 of test debt — don't. |
| `qdrant` | Vector store for hard-negative embeddings (VLM text + visual embeddings) enabling retrieval-based relabeling; too bulky in Postgres. |
| `config` + toml | 12-factor: CLI flags > env vars > `config.toml`. Factory ops wants file-based profiles per line. |
| `prometheus` | Metrics port :9090 for Grafana; process + per-crate registries. |

## 2. Models & AI

| Role | Model | Format | Runs on | License note |
|---|---|---|---|---|
| Tier-1 detector | **RF-DETR-base** (preferred) or YOLO26-S/L | ONNX fp16 (TensorRT cache) + int8 (CPU fallback) | `ort` @ CUDA/TensorRT | Apache 2.0 / Nelson's AGPL check on YOLO26 → default RF-DETR |
| Open-vocab fallback | **Grounding DINO** (text prompts from VLM) | ONNX | `ort` CPU/GPU, only on layout-change mornings | Apache 2.0 |
| Segmentation (rare) | SAM 3 prompt-se g | ONNX | Cloud or GPU, only for auto-label polish | Apache/Commercial TBC |
| Wi-Fi classifier | custom **1D-CNN + 4-layer Transformer encoder** over subcarrier×time matrix (input [64×10×32]) | ONNX fp32 | `ort` CPU quanta | project-internal |
| RL policy | small MLP (state 12-d → action 6-d) PPO/bandit head | burn `.berr` serialized | burn CPU/wgpu | internal |
| VLM (cloud) | Gemini 2.5 Pro / GPT-4o (OpenAI-compatible endpoint abstraction) | JSON-schema’d chat | `reqwest` | per-provider ToS |
| Embedding (cloud) | text-embedding / CLIP-style visual | REST | nightly only | |

## 3. Dashboard

- **Next.js 15 (App Router), TypeScript strict, Tailwind 4, shadcn/ui, TanStack Query, Zustand.**
- Charts: **Recharts** (business charts), **deck.gl** (heatmap of motion energy over floorplan PNG), **Three.js** optional 3D digital twin (phase-2 stretch).
- Video overlays: `<canvas>` 2D overlay synced to MSE/WebRTC player; boxes + tracks + crossing lines drawn from WS events.
- State machine mirrored from backend `LifecycleState` enum for UI parity.

## 4. Database & Storage

- **PostgreSQL 16 + TimescaleDB 2.x** (docker image `timescale/timescaledb:latest-pg16`).
- TimescaleDB roles: hypertables for `count_events`, `csi_metrics`, `rl_transitions`; auto `compress_segmentby = 'zone_id, camera_id'` after 7 days; retention 180d business data.
- **Redis** (or `cachefs` RAM ring): hard-negative frame ring buffer, MAX 1 GB factory-day RAM budget; eviction = FIFO by day-part.
- **Qdrant**: single-node, snapshot nightly.

## 5. External APIs / Integrations

| API | Used for | Auth |
|---|---|---|
| OpenAI-compatible Chat (GPT-4o, Gemini via gateway, Claude via OpenAI shim) | VLM diffing, root cause, auto-label | `VLM_API_KEY` env, roto-limit 30/min (firewall allow) |
| Webhooks out: ERP/MES, Slack/Teams | boot status, layout change alerts, EOD totals | HMAC-SHA256 signature header |
| on-prem REST from ERP (pull, optional tonic gRPC) | shift calendar, SKU list for ROI naming | mTLS optional |

## 6. Hardware Reference (validated builds)

| Role | Config |
|---|---|
| **Factory Edge Server** | x86 + NVIDIA (12GB+ VRAM): e.g. i7 + RTX 4060/T4/3050-class or Jetson Orin NX. 32 GB RAM. NVMe PCI RVMA ≤ 4 TB (ring buffer chapter). Ubuntu 24.04, CUDA 12.x, ONNX Runtime GPU 1.19. |
| **CSI nodes** | ESP32-S3 (esp-csi FW) over USB-serial or UDP; or Intel AX210 NIC mini-PCs (e.g. Intel NUC) with CSI extractor tool. |
| **Network** | Cameras on isolated VLAN, MTU 1500, wired PoE. CSI VLAN = Wi-Fi APs in monitor/pomp state (esp-csi). |
| **Time** | On-edge chrony (stratum 2 local) or linuxptp for sub-ms alignment. |
