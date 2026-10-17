# 04 — CCTV Ingestion Crate (`ingestion-cctv`)

## 1. Public Interface

```rust
pub struct CctvIngestor { /* per-camera pipelines + shared ring */ }

impl CctvIngestor {
    /// Start N camera pipelines; returns supervising handle.
    pub async fn spawn_all(cfg: &CameraSet, ring: Arc<FrameRing>,
        events: mpsc::Sender<IngestEvent>) -> Result<CctvSupervisor, IngestError>;

    /// One-shot high-res snapshot for morning calibration (bypasses ring).
    pub async fn capture_snapshot(&self, camera_id: CameraId) -> Result<Snapshot, IngestError>;
}

pub enum IngestEvent {
    FrameReady { camera_id, ts_edge: EdgeStamp, frame: FrameView },
    LuminanceCrossed { camera_id, ts_edge, lux_estimate: f32, direction: CrossDir },
    StreamDegraded { camera_id, reason: DegradationReason },
    StreamLost { camera_id, since: DateTime<Utc> },
}

pub struct FrameRing { /* rtrb SPSC per camera, drop-oldest, ~8s @30fps low-res + 2s full-res */ }
```

## 2. GStreamer Pipeline

Per camera (NVDEC hardware path):

```
rtspsrc location=rtsp://... latency=100 protocols=tcp
 ! rtph264depay ! h264parse ! nvh264dec ! nvvidconv ! video/x-raw(memory:NVMM),format=NV12
 ! tee name=t
 t. ! queue ! videoconvert ! video/x-raw,format=BGRx appsink (→ FrameRing, downscale 640 for infer)
 t. ! queue ! appsink hi-res (→ snapshot/luminance path, 1080p, 1 fps)
```

Luminance trigger: compute on a **64×36 downsample** of every frame:

```
V = 0.2126 R + 0.7152 G + 0.0722 B  (BT.709 luma)
frame_lux_proxy = mean(V) * calibration_gain[camera]
```

- Sustain a rolling 3-min baseline during Standby; trigger state change when `lux_proxy > threshold_on (≈ 25)` sustained ≥ 10 s **on ≥ 2/3 cameras** → send `LuminanceCrossed{On}`.
- Never rely on a single camera; ingress with hysteresis (on=25, off=12, dwell=5 min) to prevent flicker flapping.

## 3. Robustness Rules

1. `rtspsrc` `drop-on-latency=true`, `latency=100`; rtpjitterbuffer drops late frames rather than buffering → stable ≤150 ms decode latency budget.
2. Watchdog: if no `video` sample in **5 s** → pipeline restart with backoff `2^n ×1s` capped 60 s. If bus error repeats 3× in 10 min → `StreamLost` event + webhook alert.
3. Pipeline caps: `max-bytes=1_000_000`, `sync=false` for appsink (we own pacing).
4. Ring: `FrameRing` = per-camera `rtrb::RingBuffer<FrameView>` (unusual: our consumers are SPSC single infer thread, so SPSC-style alignment). Overflow → drop-oldest manually with `dropped_frames` metric; never block the gst callback thread.
5. Snapshot path: request a `GstSample` from the hi-res sink directly via async bridge (`gst-oneshot-cmd` channel) — do not reuse the ring.

## 4. Golden Baseline & SSIM (morning)

```rust
pub struct GoldenBaseline { camera_id, capture_ts, jpg: Vec<u8>, ssim_ref_luma: Mat }
```

- Captured each day end-of-day or first morning boot; stored under `data/baselines/{camera_id}/{yyyy-mm-dd}.jpg`.
- Morning diff: `ssim(luma_today, luma_yesterday)` on a 3×3 block grid → a score map; blocks with `Δssim > 0.12` shipped to VLM with bounding boxes promoted to the VLM prompt (see docs/09 §3).
- Storage cost: 1–2 MB/day/camera; retention 90 days, then delete.

## 5. Timestamping (contract with common-types)

```rust
pub struct EdgeStamp {
    pub utc: DateTime<Utc>,          // from PTP/NTP-corrected clock at edge
    pub stream_pts: u64,             // gst PTS (host.monotonic base), used for intra-stream ordering
    pub ptp_offset_ns: i64,          // measured skew (monitoring only); alarm if > 25 ms
}
```

RTSPP latency trick: we do not trust camera-side timestamps; we stamp at decode. If PTP chrony is down for >2 s, `EdgeStamp::clock_ok=false` and fusion falls back to divergence-forced uncertainty.

## 6. Config (partial TOML)

```toml
[cameras.line2_cam1]
url = "rtsp://10.42.7.11:554/stream"
zone_id = "line_2"
roi_file = "rois/line2_cam1.toml"     # virtual counting line vectors, editable in dashboard
luma_gain = 7.2                      # proxy LUX calibration (bench measured with lux meter)
decode_gpu = "nvdec"
```

## 7. Testing Hooks

- `CctvIngestor` must accept a `SourceAdapter` trait; CI substitutes a **file-backed SourceAdapter** (H.264 fixture replay at 30fps) — see docs/14 §3 "camera fixture replay".
- Unit tests demand: luminance hysteresis, golden SSIM block math, ring drop-oldest semantics, GStreamer bus error mapping.
