-- Shado core schema (docs/11). Plain PostgreSQL-first; every TimescaleDB
-- statement below is guarded so the extension being absent degrades to
-- plain tables instead of failing the boot.

DO $$ BEGIN
  CREATE EXTENSION IF NOT EXISTS timescaledb;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'timescaledb extension unavailable; refusing hypertables';
END $$;

CREATE TABLE zones (
  zone_id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  camera_ids TEXT[] NOT NULL DEFAULT '{}',
  csi_node_ids SMALLINT[] NOT NULL DEFAULT '{}',
  roi_json JSONB NOT NULL DEFAULT '[]',
  throughput_model JSONB,
  retired_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE calibration_state (
  id UUID PRIMARY KEY,
  ts TIMESTAMPTZ NOT NULL,
  lifecycle TEXT NOT NULL,
  reason TEXT,
  vlm_diff JSONB,
  roi_changes JSONB,
  cameras_ok SMALLINT,
  cameras_total SMALLINT,
  csi_nodes_ok SMALLINT,
  csi_nodes_total SMALLINT
);
CREATE INDEX calibration_state_ts_desc ON calibration_state (ts DESC);

CREATE TABLE count_events (
  event_id UUID NOT NULL,
  zone_id TEXT NOT NULL REFERENCES zones,
  ts_from TIMESTAMPTZ NOT NULL,
  ts_to TIMESTAMPTZ NOT NULL,
  count_delta INT NOT NULL,
  cumulative_after BIGINT NOT NULL,
  confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  uncertainty_mask SMALLINT NOT NULL,
  modality_vision REAL NOT NULL,
  modality_wifi REAL NOT NULL,
  conf_threshold REAL,
  iou_threshold REAL,
  track_buffer SMALLINT,
  track_ids BIGINT[],
  model_version TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (event_id, ts_from)
);
CREATE INDEX count_events_zone_ts ON count_events (zone_id, ts_from DESC);

-- TimescaleDB promotion (hypertable + policies + continuous aggregate).
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'timescaledb') THEN
    PERFORM create_hypertable('count_events', 'ts_from', chunk_time_interval => INTERVAL '1 day', if_not_exists => TRUE);
    PERFORM add_retention_policy('count_events', INTERVAL '180 days', if_not_exists => TRUE);
    ALTER TABLE count_events SET (
      timescaledb.compress = true,
      timescaledb.compress_segmentby = 'zone_id',
      timescaledb.compress_orderby = 'ts_from DESC');
    PERFORM add_compression_policy('count_events', INTERVAL '7 days', if_not_exists => TRUE);
  ELSE
    RAISE NOTICE 'timescaledb not available; count_events stays a plain table';
  END IF;
END $$;

CREATE TABLE uncertain_snapshots (
  frame_id UUID PRIMARY KEY,
  zone_id TEXT NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  camera_id TEXT NOT NULL,
  jpeg_lowres BYTEA,
  uncertainty_mask SMALLINT NOT NULL,
  divergence REAL,
  state JSONB NOT NULL,
  promotion TEXT NOT NULL DEFAULT 'Amber',
  vlm_root_cause TEXT,
  coco_annotations JSONB,
  reviewed_by TEXT,
  reviewed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX uncertain_snapshots_promotion_ts ON uncertain_snapshots (promotion, ts DESC);
CREATE INDEX uncertain_snapshots_zone_ts ON uncertain_snapshots (zone_id, ts DESC);

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'timescaledb') THEN
    PERFORM create_hypertable('csi_metrics', 'ts', if_not_exists => TRUE);
    PERFORM add_retention_policy('csi_metrics', INTERVAL '90 days', if_not_exists => TRUE);
  END IF;
END $$;

CREATE TABLE csi_metrics (
  node_id SMALLINT NOT NULL,
  zone_id TEXT NOT NULL,
  ts TIMESTAMPTZ NOT NULL,
  doppler_energy REAL,
  motion_variance REAL,
  presence_prob REAL,
  mover_count REAL,
  packet_loss_ratio REAL,
  ptp_offset_ns BIGINT,
  PRIMARY KEY (node_id, ts)
);

CREATE TABLE audit_corrections (
  correction_id UUID PRIMARY KEY,
  zone_id TEXT NOT NULL,
  reported_count BIGINT NOT NULL,
  actual_count BIGINT NOT NULL,
  ts_from TIMESTAMPTZ NOT NULL,
  ts_to TIMESTAMPTZ NOT NULL,
  failure_tags SMALLINT NOT NULL,
  notes TEXT,
  operator_id TEXT NOT NULL,
  outcome TEXT NOT NULL DEFAULT 'submitted',
  reward_computed REAL,
  retrain_requested BOOLEAN NOT NULL DEFAULT false,
  root_cause_report JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX audit_corrections_zone_ts ON audit_corrections (zone_id, ts_from DESC);

CREATE TABLE rl_audit (
  transition_id UUID PRIMARY KEY,
  ts TIMESTAMPTZ NOT NULL,
  zone_id TEXT,
  "before" JSONB NOT NULL,
  "after" JSONB NOT NULL,
  reward REAL,
  was_clamped BOOLEAN NOT NULL,
  rejected BOOLEAN NOT NULL DEFAULT false,
  reason TEXT NOT NULL,
  promoted_model TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rl_audit_ts_desc ON rl_audit (ts DESC);

CREATE TABLE model_registry (
  version TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('detector', 'csi', 'rl')),
  path TEXT NOT NULL,
  checksum TEXT NOT NULL,
  size_bytes BIGINT,
  train_dataset_ref TEXT,
  eval_report JSONB,
  status TEXT NOT NULL DEFAULT 'staged',
  activated_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE webhook_outbox (
  event_id UUID PRIMARY KEY,
  channel TEXT NOT NULL,
  url TEXT NOT NULL,
  payload JSONB NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts SMALLINT NOT NULL DEFAULT 0,
  next_retry_at TIMESTAMPTZ
);
CREATE INDEX webhook_outbox_status ON webhook_outbox (status, next_retry_at);
