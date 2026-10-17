# 13 — Deployment & Operations

## 1. Deployment modes

| Mode | Target | Notes |
|---|---|---|
| **Factory edge box** (primary) | x86 + NVIDIA GPU on-prem | docker compose; nvidia-container-toolkit; TimescaleDB, Qdrant, engine containers |
| **Jetson Orin** (pilot, 2 cams max) | secondary | CPU EP fallback for CSI model, TensorRT for detection |
| **Dev/CI** | laptop | synthetic sources only (see docs/14); no GPU required |

## 2. `docker-compose.yml` sketch (prod)

```yaml
services:
  engine:
    image: registry/shado/engine:${TAG}
    env_file: .env.production
    deploy: { resources: { reservations: { devices: [{ driver: nvidia, count: 1, capabilities: [gpu] }] } } }
    volumes: [ "./config:/app/config:ro", "data:/app/data" ]
    restart: unless-stopped
    ports: ["8080:8080", "9090:9090"]
  db:
    image: timescale/timescaledb:latest-pg16
    environment: { POSTGRES_PASSWORD: ${DB_PASSWORD} }
    volumes: [ "dbdata:/var/lib/postgresql/data" ]
    command: [ "postgres", "-c", "shared_buffers=2GB", "-c", "max_wal_size=4GB" ]
  qdrant:
    image: qdrant/qdrant:latest
    volumes: [ "qdrantdata:/qdrant/storage" ]
volumes: { dbdata: {}, qdrantdata: {}, data: {} }
```

## 3. Time synchronization (MANDATORY)

- Edge box runs `chrony` (local stratum: upstream corporate NTP + GPS if available); `ptp4l`/`linuxptp` optional for sub-ms when switch supports PTP.
- **Alarm** if offset > 25 ms or sync lost > 2 s → `ClockDegraded` state (fusion tolerance widens; report shows the banner).
- CCTV frames are stamped **at decode** (not from camera RTSPP header) — this is the authoritative edge clock.

## 4. Storage budgets (per factory defaults, 6 cams, 2 CSI nodes × 8 zones)

| Store | Budget | Retention |
|---|---|---|
| RAM frame ring | 1 GB | ~8 s |
| NVMe uncertain ring + snapshots | 40 GB | 7 days |
| Postgres/Timescale count+csi+rl | 2 GB | 90–180 days (compression policy) |
| Baseline jpgs | 2 GB | 90 days |
| Qdrant | 2 GB | forever (snapshot weekly) |
| VLM labeled dataset export | 5 GB | 1 year |

Disk alarm at 85%; engine degrades before OOM by pausing snapshot capture.

## 5. Security posture

- Factory LAN only: RTSP VLAN, CSI VLAN; only two outbound egress points: **VLM API**, **webhooks** — enforced at firewall allow-list.
- Secrets: env-only, secret store (Doppler/1Password/team vault or `.env` + chmod 600 as baseline). Never log tokens; tracing redaction filter configured for `api[_-]?key|bearer`.
- Dashboard: SSO (factory AD) or magic-link; role gates: operator (audit), manager (audit+report+rois), admin (policy, models, admin endpoints).
- Webhook signing HMAC; inbound admin endpoints need mTLS or loopback-only binding.

## 6. Observability

- Prometheus `/metrics` — engine, per-camera inf loop, fusion, RL, VLM circuit, DB lag, webhook queue.
- Grafana dashboards: "Factory Health", "Fusion Quality", "RL Runway" (guardrail distance), "VLM Spend".
- Logs: JSON via `tracing-subscriber`, one log line per structural event; `RUST_LOG=info` prod default, `shado*=debug` per-runtime on regression hunts.
- Sentry optional (Rust `sentry-tracing`) for panics only — no PII.

## 7. Secret/reference env vars

```
SHADO_CONFIG=/app/config/factory_a.toml
DATABASE_URL=postgres://shado:...@db:5432/shado
QDRANT_URL=http://qdrant:6334
VLM_PROVIDER=openai|gemini
VLM_API_BASE=...           # allow-listed gateway
VLM_API_KEY=...
VLM_MODEL=gpt-4o|gemini-2.5-pro
VLM_DAILY_USD_CAP=15
WEBHOOK_SECRET=...         # HMAC key
JWT_SECRET=...             # dashboard sessions
ADMIN_TOKEN=...            # /admin endpoints
```

## 8. Runbook — quick operators' card

| Situation | Action |
|---|---|
| Engine won't count after boot | Check `/readyz`; then `/api/v1/lifecycle`; if cameras_ok < required, check RTSP VLAN. `docker compose restart engine` safe (state restores). |
| Clock alarm fired for > 10 min | Stop counting in that window; check chrony; corrections later must use "ClockDegraded" tag. |
| One camera dead | System auto-degrades to CSI-only for that zone (in-UI red tile); replace camera; on return, morning golden baseline re-captures. |
| Count grossly wrong today | Submit `HumanAuditFeedback` with retrain=true — nightly pipeline trains; verify morning report's `model_registry.version` changed & shadow passed. |
| RL "reject" webhook | → docs/08 §6; verify "Pause policy" switched off after investigating. |
| Disk > 85% | Stop captures auto via engine; free NVMe (`data/uncertain` retention policy runbook SQL). |
| Emergency manual stop | `POST /api/v1/lifecycle/force -f Standby` (admin), then disconnect cameras at switch level. |
