//! Wire types mirroring the Rust backend (docs/10 §3, crates/api-server/src/ws.rs).
//! Field names are snake_case exactly as serde serializes them.

export type UncertaintyFlag =
  | "OCCLUDED_ESTIMATE"
  | "MODALITY_DIVERGENCE"
  | "CLOCK_DEGRADED"
  | "LOW_TRACK_STRENGTH";

export interface CountEvent {
  event_id: string;
  zone_id: string;
  ts_from: string;
  ts_to: string;
  count_delta: number;
  cumulative_after: number;
  confidence: number;
  /** bitflags Display: "OCCLUDED_ESTIMATE | MODALITY_DIVERGENCE" (or masked int). */
  uncertainty: string;
  modality_blend: { vision: number; wifi: number };
  thresholds_used: {
    conf_threshold: number;
    iou_threshold: number;
    track_buffer_frames: number;
  };
  track_ids: number[];
  model_version: string;
}

export interface FusionSignal {
  zone_id: string;
  ts: string;
  vision_rate: number;
  wifi_rate: number;
  divergence: number;
}

export interface RlTransition {
  before: { conf_threshold: number; iou_threshold: number; vision_weight: number; track_buffer_frames: number };
  after: { conf_threshold: number; iou_threshold: number; vision_weight: number; track_buffer_frames: number };
  reason: string;
  rejected: boolean;
}

export interface LifecycleEvent {
  from: string;
  to: string;
  ts: string;
}

export interface CameraHealth {
  camera_id: string;
  ok: boolean;
  fps: number;
  dropped_ratio: number;
}

export type WsPayload =
  | CountEvent
  | FusionSignal
  | RlTransition
  | LifecycleEvent
  | CameraHealth
  | { frame_id: string; ts: string; zone_id: string };

export type WsEnvelope = { kind: string; payload: WsPayload };

/** GET /api/v1/rl/state */
export interface RlState {
  paused_until: string | null;
  override_note: string | null;
  rejected_24h: number;
}

/** GET /api/v1/audit/pending → {"pending": rows[]} */
export interface PendingWindow {
  window_id: string;
  zone_id: string;
  ts_from: string;
  ts_to: string;
  predicted_count: number;
  confidence: number;
  reason: string;
}

export interface CorrectionSubmitBody {
  zone_id: string;
  reported_count: number;
  actual_count: number;
  ts_from: string;
  ts_to: string;
  failure_tags: number;
  notes: string | null;
  operator_id: string;
  wants_retrain: boolean;
}

export interface CorrectionResult {
  correction_id: string;
  reward_computed: number;
  retrain: boolean;
}
