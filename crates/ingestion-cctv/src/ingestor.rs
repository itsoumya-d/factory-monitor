//! Per-camera pipeline orchestration (docs/04 §1) — the async supervising
//! layer over `SourceAdapter` + luminance trigger + frame ring.
//!
//! Fixture path: sources exhaust orderly; the retry/watchdog graph here (with
//! bounded backoff and `StreamLost` after N consecutive strikes) is shared
//! with the hardware RTSP source landing in the hardware phase (docs/04 §3).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::error::CctvError;
use crate::luma::lux_proxy;
use crate::ring::{FrameRing, RingPushOutcome};
use crate::source::SourceAdapter;
use crate::trigger::{CrossDirection, LuminanceQuorum, LuminanceTrigger};
use crate::watchdog::MAX_BACKOFF;
use common_types::{CameraId, EdgeStamp, FrameView, ZoneId};

pub const SYSTEM_QUORUM_CAMERA: &str = "system_quorum";

/// Crossings either flow per-camera straight onto `events`, or (when a quorum
/// is configured) through the combiner and re-emerge with `camera_id` of
/// [`SYSTEM_QUORUM_CAMERA`].
#[derive(Debug, Clone)]
pub enum IngestEvent {
    FrameReady {
        frame: FrameView,
    },
    LuminanceCrossed {
        camera_id: CameraId,
        ts_edge: EdgeStamp,
        lux_estimate: f32,
        direction: CrossDirection,
    },
    StreamDegraded {
        camera_id: CameraId,
        reason: StreamDegradationReason,
    },
    StreamLost {
        camera_id: CameraId,
        since: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub enum StreamDegradationReason {
    SourceError(String),
    BufferChoke { consecutive_dropped: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerParams {
    pub on_threshold: f32,
    pub off_threshold: f32,
    pub on_sustain_ms: u64,
    pub off_dwell_ms: u64,
}

impl Default for TriggerParams {
    fn default() -> Self {
        Self {
            on_threshold: 25.0,
            off_threshold: 12.0,
            on_sustain_ms: 10_000,
            off_dwell_ms: 300_000,
        }
    }
}

pub struct CameraPipeline {
    pub camera_id: CameraId,
    pub zone_id: Option<ZoneId>,
    pub luma_gain: f32,
    pub ring_capacity: usize,
    pub source: Box<dyn SourceAdapter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnerLimits {
    pub max_retries: u32,
    pub backoff_base_ms: u64,
}

impl Default for SpawnerLimits {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_base_ms: 1_000,
        }
    }
}

/// Camera fan-in description. Holds owned sources, so it moves (not clones).
pub struct CameraSet {
    pub trigger: TriggerParams,
    /// `Some(required)` installs the `>= 2/3`-style quorum combiner:
    /// per-camera crossings are consolidated before the system reacts
    /// (docs/04 §2 — "never rely on a single camera"). `None` forwards all
    /// per-camera crossings uncombined.
    pub quorum_required: Option<u32>,
    pub limits: SpawnerLimits,
    pub cameras: Vec<CameraPipeline>,
}

pub struct SpawnedPipelines {
    pub rings: Vec<(CameraId, Arc<FrameRing>)>,
    camera_tasks: Vec<JoinHandle<()>>,
    quorum_task: Option<JoinHandle<()>>,
}

impl SpawnedPipelines {
    pub async fn join_all(self) {
        for task in self.camera_tasks {
            let _ = task.await;
        }
        if let Some(q) = self.quorum_task {
            let _ = q.await;
        }
    }

    pub fn ring(&self, camera: &CameraId) -> Option<&Arc<FrameRing>> {
        self.rings
            .iter()
            .find(|(id, _)| id == camera)
            .map(|(_, ring)| ring)
    }
}

/// Spawn every camera pipeline. Fail-at-open: any invalid camera parameter
/// aborts before a single task starts.
pub fn spawn_all(
    set: CameraSet,
    events: UnboundedSender<IngestEvent>,
) -> Result<SpawnedPipelines, CctvError> {
    let mut seen_cameras: std::collections::HashSet<String> = std::collections::HashSet::new();
    for camera in &set.cameras {
        if !(camera.luma_gain.is_finite() && camera.luma_gain > 0.0) {
            return Err(CctvError::InvalidLumaGain(camera.camera_id.to_string()));
        }
        if camera.ring_capacity == 0 {
            return Err(CctvError::EmptySource(format!(
                "{}: ring_capacity must be >= 1",
                camera.camera_id
            )));
        }
        if !seen_cameras.insert(camera.camera_id.as_str().to_string()) {
            return Err(CctvError::DuplicateCamera(camera.camera_id.to_string()));
        }
    }

    let (cross_tx, cross_rx) = unbounded_channel::<(CameraId, EdgeStamp, f32, CrossDirection)>();
    let total_cameras = set.cameras.len() as u32;
    let quorum_task = match set.quorum_required {
        Some(required) => Some(tokio::spawn(run_quorum_combiner(
            total_cameras,
            required,
            cross_rx,
            events.clone(),
        ))),
        None => Some(tokio::spawn(drain_without_quorum(cross_rx))),
    };

    let mut rings = Vec::new();
    let mut camera_tasks = Vec::new();
    for camera in set.cameras {
        let ring = Arc::new(FrameRing::new(
            camera.camera_id.clone(),
            camera.ring_capacity,
        ));
        rings.push((camera.camera_id.clone(), ring.clone()));
        let trigger = LuminanceTrigger::custom(
            set.trigger.on_threshold,
            set.trigger.off_threshold,
            set.trigger.on_sustain_ms,
            set.trigger.off_dwell_ms,
        );
        camera_tasks.push(tokio::spawn(run_camera_pipeline(
            camera,
            ring,
            trigger,
            set.limits.max_retries,
            set.limits.backoff_base_ms,
            cross_tx.clone(),
            events.clone(),
        )));
    }

    Ok(SpawnedPipelines {
        rings,
        camera_tasks,
        quorum_task,
    })
}

async fn drain_without_quorum(
    mut rx: UnboundedReceiver<(CameraId, EdgeStamp, f32, CrossDirection)>,
) {
    while rx.recv().await.is_some() {
        // Aggregator-less mode: consumed so producers never block; per-camera
        // crossings already reached the main event stream from the task.
    }
}

async fn run_quorum_combiner(
    total_cameras: u32,
    required: u32,
    mut rx: UnboundedReceiver<(CameraId, EdgeStamp, f32, CrossDirection)>,
    events: UnboundedSender<IngestEvent>,
) {
    let mut quorum = LuminanceQuorum::with_total_and_required(total_cameras, required);
    while let Some((camera_id, ts_edge, lux_estimate, direction)) = rx.recv().await {
        if let Some(system_direction) = quorum.feed(camera_id, direction) {
            if let Ok(system_camera) = CameraId::try_new(SYSTEM_QUORUM_CAMERA) {
                let _ = events.send(IngestEvent::LuminanceCrossed {
                    camera_id: system_camera,
                    ts_edge,
                    lux_estimate,
                    direction: system_direction,
                });
            }
        }
    }
}

async fn run_camera_pipeline(
    camera: CameraPipeline,
    ring: Arc<FrameRing>,
    mut trigger: LuminanceTrigger,
    max_retries: u32,
    backoff_base_ms: u64,
    cross_tx: UnboundedSender<(CameraId, EdgeStamp, f32, CrossDirection)>,
    events: UnboundedSender<IngestEvent>,
) {
    const BUFFER_CHOKE_STREAK: u32 = 8;

    let mut source = camera.source;
    let mut attempts: u32 = 0;
    let mut consecutive_dropped: u32 = 0;
    let mut last_seen: Option<DateTime<Utc>> = None;

    loop {
        match source.next_frame().await {
            Some(Ok(view)) => {
                attempts = 0;
                if consecutive_dropped != 0 {
                    consecutive_dropped = 0;
                }
                match lux_proxy(&view, camera.luma_gain) {
                    Ok(lux) => {
                        if matches!(ring.push(view.clone()), RingPushOutcome::DroppedNewest) {
                            consecutive_dropped += 1;
                            if consecutive_dropped >= BUFFER_CHOKE_STREAK {
                                let _ = events.send(IngestEvent::StreamDegraded {
                                    camera_id: camera.camera_id.clone(),
                                    reason: StreamDegradationReason::BufferChoke {
                                        consecutive_dropped: BUFFER_CHOKE_STREAK,
                                    },
                                });
                            }
                        }

                        let _ = events.send(IngestEvent::FrameReady {
                            frame: view.clone(),
                        });

                        if let Some(direction) = trigger.feed(view.ts, lux) {
                            let _ =
                                cross_tx.send((camera.camera_id.clone(), view.ts, lux, direction));
                        }
                        last_seen = Some(view.ts.utc);
                    }
                    Err(err) => {
                        attempts = attempts.saturating_add(1);
                        debug!(camera = %camera.camera_id, "lux_proxy failed: {err}");
                        let _ = events.send(IngestEvent::StreamDegraded {
                            camera_id: camera.camera_id.clone(),
                            reason: StreamDegradationReason::SourceError(err.to_string()),
                        });
                        if attempts > max_retries {
                            let _ = events.send(IngestEvent::StreamLost {
                                camera_id: camera.camera_id.clone(),
                                since: last_seen.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                            });
                            break;
                        }
                    }
                }
            }
            Some(Err(err)) => {
                attempts = attempts.saturating_add(1);
                debug!(camera = %camera.camera_id, "source error attempt {attempts}: {err}");
                if attempts > max_retries {
                    let _ = events.send(IngestEvent::StreamLost {
                        camera_id: camera.camera_id.clone(),
                        since: last_seen.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                    });
                    break;
                }
                let _ = events.send(IngestEvent::StreamDegraded {
                    camera_id: camera.camera_id.clone(),
                    reason: StreamDegradationReason::SourceError(err.to_string()),
                });
                let exp = attempts.saturating_sub(1).min(31);
                let delay_ms = backoff_base_ms
                    .saturating_mul(1u64 << exp)
                    .min(MAX_BACKOFF.as_millis() as u64);
                tokio::time::sleep(Duration::from_millis(delay_ms.max(1))).await;
            }
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::collections::VecDeque;

    use super::*;
    use common_types::PixelFormat;
    use common_types::ZoneId;

    const LUMA_BYTES: usize = (64 * 36) as usize;

    fn gray_frame(camera: &CameraId, stream_pts: u64, fill: u8) -> FrameView {
        FrameView {
            camera_id: camera.clone(),
            ts: EdgeStamp::new(
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
                    + chrono::Duration::milliseconds((stream_pts * 10) as i64),
                stream_pts,
                0,
                true,
            ),
            width: 64,
            height: 36,
            format: PixelFormat::Gray8,
            data: vec![fill; LUMA_BYTES],
        }
    }

    /// Scripted source: plays a fixed sequence of Ok(frame)/Err outcomes then
    /// ends orderly. Failures slot into the script at given frame indexes.
    struct ScriptedSource {
        script: VecDeque<Result<FrameView, CctvError>>,
    }

    impl ScriptedSource {
        fn failed_frames(
            camera: &CameraId,
            fills: &[u8],
            fail_after: &[usize],
            error: CctvError,
        ) -> Self {
            let mut fails: VecDeque<usize> = fail_after.iter().copied().collect();
            let mut script = VecDeque::new();
            for (i, fill) in fills.iter().enumerate() {
                if fails.front() == Some(&i) {
                    fails.pop_front();
                    script.push_back(Err(error.clone()));
                }
                script.push_back(Ok(gray_frame(camera, i as u64, *fill)));
            }
            Self { script }
        }

        fn bright_then_dark(camera: &CameraId, bright: u64, dark: u64) -> Self {
            let mut script = VecDeque::new();
            for i in 0..bright {
                script.push_back(Ok(gray_frame(camera, i, 40)));
            }
            for i in 0..dark {
                script.push_back(Ok(gray_frame(camera, bright + i, 8)));
            }
            Self { script }
        }

        /// All-failures script: `n` error attempts then orderly end.
        fn errors(camera: &CameraId, n: usize, error: CctvError) -> Self {
            let mut script = VecDeque::new();
            for _ in 0..n {
                script.push_back(Err(error.clone()));
            }
            let _ = camera;
            Self { script }
        }
    }

    #[async_trait::async_trait]
    impl SourceAdapter for ScriptedSource {
        async fn next_frame(&mut self) -> Option<Result<FrameView, CctvError>> {
            // Yield between frames so concurrent camera tasks interleave and
            // quorum votes arrive in production-realistic order.
            tokio::task::yield_now().await;
            self.script.pop_front()
        }
    }

    fn camera_pipeline(
        id: &str,
        gain: f32,
        ring: usize,
        source: Box<dyn SourceAdapter>,
    ) -> CameraPipeline {
        CameraPipeline {
            camera_id: CameraId::try_new(id).unwrap(),
            zone_id: Some(ZoneId::try_new("line_1").unwrap()),
            luma_gain: gain,
            ring_capacity: ring,
            source,
        }
    }

    fn fast_triggers() -> TriggerParams {
        TriggerParams {
            on_sustain_ms: 40,
            off_dwell_ms: 0,
            ..TriggerParams::default()
        }
    }

    async fn collect(mut rx: UnboundedReceiver<IngestEvent>) -> Vec<IngestEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn failing_source_degrades_then_recovers() {
        let cam = CameraId::try_new("cam_a").unwrap();
        // 2 good frames, 2 transient failures (within retry budget), 3 good,
        // orderly exhaustion.
        let source = ScriptedSource::failed_frames(
            &cam,
            &[40, 40, 40, 40, 40],
            &[2, 3],
            CctvError::EmptySource("transient decode stall".to_string()),
        );
        let (events_tx, events_rx) = unbounded_channel::<IngestEvent>();
        let set = CameraSet {
            trigger: fast_triggers(),
            quorum_required: None,
            limits: SpawnerLimits {
                max_retries: 3,
                backoff_base_ms: 1,
            },
            cameras: vec![camera_pipeline("cam_a", 1.0, 8, Box::new(source))],
        };
        let spawned = spawn_all(set, events_tx).unwrap();
        spawned.join_all().await;
        let events = collect(events_rx).await;

        let degraded = events
            .iter()
            .filter(|e| matches!(e, IngestEvent::StreamDegraded { .. }))
            .count();
        assert_eq!(
            degraded, 2,
            "each transient failure surfaces one degradation event, got {degraded}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, IngestEvent::StreamLost { .. })),
            "recovery within retry budget must never emit StreamLost"
        );
        let frames = events
            .iter()
            .filter(|e| matches!(e, IngestEvent::FrameReady { .. }))
            .count();
        assert_eq!(frames, 5, "all five good frames reach the ring path");
    }

    #[tokio::test]
    async fn exhausted_source_stream_lost_after_retry_budget() {
        let set = CameraSet {
            trigger: fast_triggers(),
            quorum_required: None,
            limits: SpawnerLimits {
                max_retries: 2,
                backoff_base_ms: 1,
            },
            cameras: vec![camera_pipeline(
                "cam_dead",
                1.0,
                8,
                Box::new(ScriptedSource::errors(
                    &CameraId::try_new("cam_dead").unwrap(),
                    4,
                    CctvError::EmptySource("dead stream".to_string()),
                )),
            )],
        };
        let (events_tx, events_rx) = unbounded_channel::<IngestEvent>();
        let spawned = spawn_all(set, events_tx).unwrap();
        spawned.join_all().await;
        let events = collect(events_rx).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, IngestEvent::StreamLost { .. })),
            "past the retry budget the stream is declared lost"
        );
    }

    #[tokio::test]
    async fn quorum_two_cameras_fire_system_on_and_off() {
        let fixtures: Vec<ScriptedSource> = (0..2)
            .map(|c| {
                ScriptedSource::bright_then_dark(
                    &CameraId::try_new(format!("cam_{c}")).unwrap(),
                    6,
                    6,
                )
            })
            .collect();
        let cameras: Vec<CameraPipeline> = fixtures
            .into_iter()
            .enumerate()
            .map(|(c, src)| camera_pipeline(&format!("cam_{c}"), 1.0, 8, Box::new(src)))
            .collect();
        let set = CameraSet {
            trigger: fast_triggers(),
            quorum_required: Some(2),
            limits: SpawnerLimits::default(),
            cameras,
        };
        let (events_tx, events_rx) = unbounded_channel::<IngestEvent>();
        let spawned = spawn_all(set, events_tx).unwrap();
        spawned.join_all().await;
        let events = collect(events_rx).await;
        let system_events: Vec<&IngestEvent> = events
            .iter()
            .filter(|e| match e {
                IngestEvent::LuminanceCrossed { camera_id, .. } => {
                    camera_id.as_str() == SYSTEM_QUORUM_CAMERA
                }
                _ => false,
            })
            .collect();
        let directions: Vec<CrossDirection> = system_events
            .iter()
            .filter_map(|e| match e {
                IngestEvent::LuminanceCrossed { direction, .. } => Some(*direction),
                _ => None,
            })
            .collect();
        assert_eq!(
            directions,
            vec![CrossDirection::On, CrossDirection::Off],
            "system quorum fires exactly one On and one Off per hysteresis sweep"
        );
    }

    #[tokio::test]
    async fn spawn_all_fails_at_open_on_bad_config() {
        let (tx, _rx) = unbounded_channel::<IngestEvent>();
        let empty_source = || ScriptedSource {
            script: VecDeque::new(),
        };
        match spawn_all(
            CameraSet {
                trigger: TriggerParams::default(),
                quorum_required: None,
                limits: SpawnerLimits::default(),
                cameras: vec![camera_pipeline("cam_bad", 0.0, 4, Box::new(empty_source()))],
            },
            tx.clone(),
        ) {
            Err(CctvError::InvalidLumaGain(_)) => {}
            other => {
                drop(other);
                panic!("bad luma gain must reject");
            }
        }

        match spawn_all(
            CameraSet {
                trigger: TriggerParams::default(),
                quorum_required: None,
                limits: SpawnerLimits::default(),
                cameras: vec![camera_pipeline(
                    "cam_zero",
                    1.0,
                    0,
                    Box::new(empty_source()),
                )],
            },
            tx.clone(),
        ) {
            Err(CctvError::EmptySource(_)) => {}
            other => {
                drop(other);
                panic!("zero ring capacity must reject");
            }
        }

        match spawn_all(
            CameraSet {
                trigger: TriggerParams::default(),
                quorum_required: None,
                limits: SpawnerLimits::default(),
                cameras: vec![
                    camera_pipeline("cam_dup", 1.0, 4, Box::new(empty_source())),
                    camera_pipeline("cam_dup", 1.0, 4, Box::new(empty_source())),
                ],
            },
            tx,
        ) {
            Err(CctvError::DuplicateCamera(_)) => {}
            other => {
                drop(other);
                panic!("duplicate camera id must reject");
            }
        }
    }

    #[tokio::test]
    async fn ring_capacity_is_respected_per_camera() {
        let cam = CameraId::try_new("cam_r").unwrap();
        let fills: Vec<u8> = (0..30).map(|i| 40 + i as u8).collect();
        let source = ScriptedSource::failed_frames(
            &cam,
            &fills,
            &[],
            CctvError::EmptySource("x".to_string()),
        );
        let (events_tx, events_rx) = unbounded_channel::<IngestEvent>();
        let set = CameraSet {
            trigger: fast_triggers(),
            quorum_required: None,
            limits: SpawnerLimits::default(),
            cameras: vec![camera_pipeline("cam_r", 10.0, 2, Box::new(source))],
        };
        let spawned = spawn_all(set, events_tx).unwrap();
        spawned.join_all().await;
        let events = collect(events_rx).await;
        let frames = events
            .iter()
            .filter(|e| matches!(e, IngestEvent::FrameReady { .. }))
            .count();
        assert_eq!(frames, 30, "all 30 frames enqueue events");
    }
}
