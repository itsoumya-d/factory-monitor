# 10 — API Server Crate (`api-server`)

axum-based. Single entrypoint binary `shado-engine`. Owns: REST, WebSocket dashboards, webhook dispatch, DB persistence, config, supervision of all other crates.

## 1. Route Map (v1)

```
GET  /healthz                                liveness (no deps)
GET  /readyz                                 readiness (db + pipelines)
GET  /metrics                                prometheus

# Lifecycle / calibration
GET  /api/v1/lifecycle                       { state, last_transition, cameras:[..], csi_nodes:[..] }
POST /api/v1/lifecycle/force                -force{state}   // ops only, audit-logged
GET  /api/v1/calibration/latest              golden baselines + VLM diff JSON
POST /api/v1/calibration/rois/{zone_id}      update counting lines (dashboard editor)

# Counts / live
GET  /api/v1/zones/{zone_id}/counts?from&to  aggregated, timescale-compressed
GET  /api/v1/zones/{zone_id}/hourly?date=    hourly curve for report
WS   /ws/live                                fan-out: CountEvent, FusionSignal, RlTransition, LifecycleEvent

# EOD audit + RL
GET  /api/v1/audit/pending                   windows flagged for operator review
POST /api/v1/audit/corrections               HumanAuditFeedback → returns correction_id + reward report
POST /api/v1/audit/corrections/{id}/commit   finalize (idempotent), 2nd step for double-confirmation of big deltas
GET  /api/v1/rl/state                        current thresholds + policy report
POST /api/v1/rl/override                     manual thresholds (PolicyAgent::pause(until))
GET  /api/v1/rl/history?date=

# Search / VLM
POST /api/v1/snapshots/query                 { zone_id, from, to, uncertainty_filter } → frame thumbnails
GET  /api/v1/labeling/queue                  auto-label review queue
POST /api/v1/labeling/{frame_id}/approve     -> COCO export mark

# Admin
POST /api/v1/admin/models/hot-swap           admin only
POST /api/v1/admin/nightly/run               manual trigger of nightly pipeline
```

## 2. Middleware stack

`tower-http` TraceId → `tracing` JSON logs; `ValidateRequest` (jsonschema on bodies); HMAC signer for outbound webhooks; per-route rate limit; request timeout 10 s; body cap 10 MB (except `/api/v1/calibration/rois` 2 MB); CORS allow dashboard origin; static passthrough of `/api/docs` (utoipa/Swagger).

## 3. WebSocket fan-out

```rust
pub enum WsEvent {
    CountEvent(PackageCountEvent),         // every crossing, ~10/s aggregate
    FusionSignal { zone_id, ts, vision_rate, wifi_rate, divergence }, // 1 Hz heartbeat
    RlTransition { before: RlAction, after: RlAction, reason: RlReason },
    LifecycleEvent(LifecycleTransition),
    CameraHealth { camera_id, ok, fps, dropped_ratio },
    LabelQueueAdded { frame_id, ts, zone_id },
}
```

Clients subscribe with `?topics=counts,rl` filter. Backpressure: `broadcast::Sender::send` with capacity 4096; slow clients are dropped (dashboard reconnects; idempotent resync via REST snapshot).

## 4. Persistence layer (sqlx)

- `sqlx::PgPool` with `max_connections=32`, statement cache LRU.
- Writes happen in a **pooled batcher** (tokio mpsc + coalescing task): counts coalesce 200 ms, telemetry 1 s — flush on flood.
- Migrations via `sqlx migrate run` (files in `migrations/`, docs/11 is the schema spec).
- Repos are trait-mocked for tests (docs/14 §4).

## 5. Outgoing webhooks (ERP/MES/Slack/Teams)

| Event | When fired | Payload |
|---|---|---|
| `FactoryBootReady` | lifecycle Counting entered | cameras, csi nodes, ts, roi_summary |
| `FactoryLayoutChanged` | VLM diff accepted | diffs, auto-applied ROI fixes, snapshot URLs |
| `DayTotalPosted` | EOD audit commit | zone totals, corrections, report URL |
| `UncertaintyAlarm` | >5 min occluded_with_fly | zone_id, since, suggested_action |
| `RlActionRejected` | guardrail rejection | reason, proposed vs clamped |

Transport: `reqwest` with 5 s timeout, 2 retries, **HMAC-SHA256** signature header `X-Shado-Signature: v1=<hex>` from `WEBHOOK_SECRET`; payload idempotency key = event_id; retry queue in table `webhook_outbox` (only retry on 5xx/timeout, not 4xx).

## 6. Config loading

- Priority: CLI > env (`SHADO_*`) > `config.toml`; profiles `config/{factory}.toml`.
- Hot reload of `daily_usd_cap`, alert thresholds via SIGHUP (tokio signal task).

## 7. Boot order (deterministic, tested)

```
1. config + tracing       2. sqlx migrations           3. clean any orphan PIDs (lockfile)
4. ring buffers           5. vlm client + circuit      6. ort pool warm
7. csi sources            8. cctv ingestors            9. aligner + fusion
10. rl agent (restore)   11. axum server             12. webhook outbox drain
13. lifecycle → Standby
```
