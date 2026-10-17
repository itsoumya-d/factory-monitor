# 11 — Database Schema (PostgreSQL 16 + TimescaleDB 2.x)

Files in `migrations/`, applied by sqlx. Naming: snake_case; all timestamps `timestamptz` UTC; UUIDv7 ids.

## 1. Tables

### `zones`
```sql
CREATE TABLE zones (
  zone_id TEXT PRIMARY KEY,              -- 'line_2'
  display_name TEXT NOT NULL,
  camera_ids TEXT[] NOT NULL DEFAULT '{}',
  csi_node_ids SMALLINT[] NOT NULL DEFAULT '{}',
  roi_json JSONB NOT NULL DEFAULT '[]',  -- virtual counting lines + masks (docs/07 §4)
  throughput_model jsonb,                -- Wi-Fi rate → pkg/s regression params
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
```

### `calibration_state`
```sql
CREATE TABLE calibration_state (
  id UUID PRIMARY KEY,                    -- uuidv7
  ts timestamptz NOT NULL,
  lifecycle TEXT NOT NULL,                -- Standby|Calibrating|Counting|Correcting|Degraded
  reason TEXT,                            -- 'lights_on' | 'manual' | 'watchdog'
  vlm_diff jsonb,                         -- full morning diff response
  roi_changes jsonb,
  cameras_ok smallint, cameras_total smallint,
  csi_nodes_ok smallint, csi_nodes_total smallint
);
CREATE INDEX ON calibration_state (ts DESC);
```

### `count_events` (hypertable)
```sql
CREATE TABLE count_events (
  event_id UUID NOT NULL,
  zone_id TEXT NOT NULL REFERENCES zones,
  ts_from timestamptz NOT NULL, ts_to timestamptz NOT NULL,
  count_delta INT NOT NULL,
  cumulative_after BIGINT NOT NULL,
  confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  uncertainty_mask SMALLINT NOT NULL,     -- bitflags
  modality_vision REAL NOT NULL, modality_wifi REAL NOT NULL,
  conf_threshold REAL, iou_threshold REAL, track_buffer SMALLINT,
  track_ids BIGINT[],
  model_version TEXT NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (event_id, ts_from)
);
SELECT create_hypertable('count_events','ts_from', chunk_time_interval => INTERVAL '1 day');
SELECT add_retention_policy('count_events', INTERVAL '180 days');
ALTER TABLE count_events SET (
  timescaledb.compress_segmentby = 'zone_id',
  timescaledb.compress_orderby = 'ts_from DESC');
SELECT add_compression_policy('count_events', INTERVAL '7 days');
CREATE INDEX ON count_events (zone_id, ts_from DESC);
```

### `hourly_rollup` (continuous aggregate)
```sql
CREATE MATERIALIZED VIEW hourly_rollup WITH (timescaledb.continuous) AS
SELECT zone_id,
       time_bucket('1 hour', ts_from) AS hour,
       sum(count_delta)::bigint AS packages,
       avg(confidence) AS avg_conf,
       sum(CASE WHEN (uncertainty_mask & 1) > 0 THEN 1 ELSE 0 END) AS occluded_events
FROM count_events GROUP BY zone_id, hour;
SELECT add_continuous_aggregate_policy('hourly_rollup',
  start_offset => INTERVAL '2 days', end_offset => INTERVAL '1 hour',
  schedule_interval => INTERVAL '1 hour');
```

### `uncertain_snapshots`
```sql
CREATE TABLE uncertain_snapshots (
  frame_id UUID PRIMARY KEY,
  zone_id TEXT NOT NULL,
  ts timestamptz NOT NULL,
  camera_id TEXT NOT NULL,
  jpeg_lowres BYTEA,              -- ≤640², ≤120 KB; ring buffer is the source of high-res if needed
  uncertainty_mask SMALLINT NOT NULL,
  divergence REAL,                -- |vision - wifi| rate gap
  state jsonb NOT NULL,           -- {tracks, detections, csi_features}
  promotion TEXT NOT NULL DEFAULT 'Amber',  -- Amber→Uploaded→AutoLabeled→LabeledOK→Trained
  vlm_root_cause TEXT,
  coco_annotations jsonb,         -- filled by auto-label pass
  reviewed_by TEXT, reviewed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON uncertain_snapshots (promotion, ts DESC);
CREATE INDEX ON uncertain_snapshots (zone_id, ts DESC);
```

### `csi_metrics` (hypertable)
```sql
CREATE TABLE csi_metrics (
  node_id SMALLINT NOT NULL,
  zone_id TEXT NOT NULL,
  ts timestamptz NOT NULL,
  doppler_energy REAL, motion_variance REAL,
  presence_prob REAL, mover_count REAL,
  packet_loss_ratio REAL, ptp_offset_ns BIGINT,
  PRIMARY KEY (node_id, ts)
);
SELECT create_hypertable('csi_metrics','ts');
SELECT add_retention_policy('csi_metrics', INTERVAL '90 days');
```

### `rl_audit`
```sql
CREATE TABLE rl_audit (
  transition_id UUID PRIMARY KEY,
  ts timestamptz NOT NULL,
  zone_id TEXT,                    -- null = global
  before jsonb NOT NULL, after jsonb NOT NULL,
  reward REAL,                     -- attributed immediate reward if any
  was_clamped BOOLEAN NOT NULL,
  rejected BOOLEAN NOT NULL DEFAULT false,
  reason TEXT NOT NULL,            -- 'eod_correction' | 'intraday_shape' | 'guardrail_reject' | 'manual_override' | 'restore'
  feedback_correction_id UUID REFERENCES audit_corrections(correction_id),
  promoted_model TEXT,             -- nightly swap info if relevant
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON rl_audit (ts DESC);
```

### `audit_corrections`
```sql
CREATE TABLE audit_corrections (
  correction_id UUID PRIMARY KEY,
  zone_id TEXT NOT NULL,
  reported_count BIGINT NOT NULL, actual_count BIGINT NOT NULL,
  ts_from timestamptz NOT NULL, ts_to timestamptz NOT NULL,
  failure_tags SMALLINT NOT NULL,  -- bitflags
  notes TEXT, operator_id TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'submitted',  -- submitted|committed|superseded
  reward_computed REAL,
  retrain_requested BOOLEAN NOT NULL,
  root_cause_report jsonb,         -- VLM localization output
  created_at timestamptz NOT NULL DEFAULT now()
);
```

### `model_registry`
```sql
CREATE TABLE model_registry (
  version TEXT PRIMARY KEY,        -- 'rfdetr-base-v3'
  kind TEXT NOT NULL CHECK (kind IN ('detector','csi','rl')),
  path TEXT NOT NULL, checksum TEXT NOT NULL, size_bytes BIGINT,
  train_dataset_ref TEXT, eval_report jsonb,
  status TEXT NOT NULL DEFAULT 'staged',   -- staged|shadow|active|retired|rollback
  activated_at timestamptz, footnote jsonb,
  created_at timestamptz NOT NULL DEFAULT now()
);
```

### `webhook_outbox`
```sql
CREATE TABLE webhook_outbox (
  event_id UUID PRIMARY KEY,
  channel TEXT NOT NULL, url TEXT NOT NULL,  -- to Cerbase, resolved from config
  payload jsonb NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',    -- pending|delivered|failed|dead
  attempts SMALLINT NOT NULL DEFAULT 0,
  next_retry_at timestamptz
);
```

## 2. Conventions

- Every critical insert writes a `tracing` span with `event_id`.
- No ON UPDATE cascades across zones; zone deletion is soft (`zones` never deleted; they are renamed `retired_<ts>`).
- All Business queries aggregate through `hourly_rollup` for the dashboard; raw `count_events` used for audit/investigation.
- migrations naming: `NNNN_do_something.sql`, standard sqlx style; tx-wrapped.
