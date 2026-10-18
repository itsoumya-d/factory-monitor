# 12 — Dashboard Spec (Next.js 15 + TypeScript)

## 1. Tech & layout

- Next.js 15 App Router, React 19, TypeScript `strict`.
- Tailwind 4 + shadcn/ui (dark/light; default dark, factory-floor high-contrast).
- Data: TanStack Query (REST) + native `WebSocket` store (Zustand) for live bucket.
- Charts: Recharts (bars/curves), deck.gl HeatmapLayer (floor plan overlay), optional Three.js `Cesanta` twin (stretch).
- i18n: English first; strings in `messages/*.json`.

## 2. Route architecture

```
/(app)
  /dashboard                 Live ops — per-zone counters, mini video tiles, fusion bars, health strip
  /reports/YYYY-MM-DD        The Visual Business Report (EOD)
  /audit                     Corrections desk: submit form + commit queue
                             (GET /audit/pending returns AuditCorrectionRow —
                             submitted corrections awaiting commit; commit
                             posts {confirm} and 409s deltas >100 without it)
  /review                    Label queue: low-res frames + boxes → approve/fix → COCO
  /settings/rois             ROI editor (drag counting lines on floor plan/camera stills)
  /settings/policy           RL knobs: pause/unpause manual override + bounds visualization
  /login                     SSO (factory AD) or magic-link; roles: operator|manager|admin
```

## 3. Components (▸ = shared)

| Component | Props | Key behavior |
|---|---|---|
| ▸ `ZoneCard` | zone_id, live count, conf, flags | flag chips: **Occluded**, **Diverged**, **ClockDegraded** |
| ▸ `MiniVideoTile` | camera_id, overlay stream | `<video>` (HLS via `/api/v1/streams/{id}/index.m3u8`, withheld if camera fails → poster) + `<canvas>` overlay of boxes/lines |
| ▸ `ConfBadge` | level 0-1 | green ≥0.9 / amber ≥0.75 / red else |
| `LiveThroughputChart` | hourly rollup | Recharts AreaChart, 24h, goal line, one series per zone |
| `BottleneckHeatmap` | zone × hour grid (`GET /api/v1/zones/{z}/hourly`) | pure DOM grid (deck.gl overlay is reserved for floorplan.png when a venue supplies one), same "calm→storm" scale, per-cell tooltip |
| `CounterTicker` | WS CountEvent | optimistic local +1, flashes on divergence |
| `CorrectionForm` | zones, windows | multi-step: pick zone → pick window → slider actual vs predicted → tags → notes → Submit. **Bad-delta guard:** if \|actual - predicted\| > 25% ask "Confirm big swing?" + requires wishRetrain checkbox |
| `SuspectWindowViewer` | window_id | timeline scrubber of 12 sampled frames + pre/post boxes + VLM root cause + operator's re-count entry |
| `LabelApprover` | frame_id, operator | `inspect` → frame bytes (`GET /api/v1/snapshots/image/{frame_id}`) + drag-to-fix boxes (SVG, normalized coords, preloaded from the queue row's `coco_annotations`) + approve → `POST /api/v1/labeling/{frame_id}/approve` body `{operator_id, coco_annotations}` (TS mirror of `vlm_orchestrator::package_coco`: 1-indexed ids, pixel bbox, fail-closed taxonomy). Reject/Teach verbs need backend routes first |
| `RlPanel` | current + history | sliders (disabled unless unlocked), runway visualization of clamps, "Pause policy" switch, bottom "Revert to last-safe" button |
| `LifecycleStrip` | state | Shows Standby/Calibrating/Counting/Correcting/Degraded with pulse animation + "what's blocking" checklist during Major durations |

## 4. Live WS contract (consumes docs/10 §3 server payloads)

Reconnect strategy: exponential backoff 1s→30s, on reconnect fetch REST snapshot first, then resume deltas; drop duplicate `event_id`s. Show stale-data banner if `last_message_age > 10s`.

## 5. Report tab (EOD interactive)

- Blocks: header (date, shift), zone grid (cards) → selection, *Right*: runs:
  1. predicted vs actual (after corrections) big numbers with delta chip
  2. hourly curve + confidence band,
  3. bottleneck heatmap,
  4. "windows to review" timeline of `LOW_CONFIDENCE` spans (click → open Viewer),
  5. buttons: `Submit corrections` → POST correction, `Export PDF/CSV`, `Copy webhook payload`.
- Shows per-window **RL reward effects** transparently: "This correction will tighten τ_conf in PM shift tomorrow from 0.44 → 0.51".

## 6. Accessibility & i18n & theming

- WCAG AA: all interactive elements ≥ 44px touch (gloved-factory touch panels).
- High-contrast mode + keyboard nav for audit tab (power users submit 20+ corrections/day).
- No flashing content > 3/s (photosensitivity) except confirmation moments (< 600 ms).
- All numbers right-aligned monospace; variation highlighted.

## 7. Testing (mirrors docs/14 §9)

- Component tests: Playwright for correction form + ROI editor E2E; Vitest/RTL for units.
- **WS mock harness** replays synthetic event streams from fixture JSON files (backend fixture day).
- Visual regression: Playwright screenshot diffs on dark/light, CRITIC on the audit tab.

## 9. Grounded implementation notes (docs/10 backend parity)

- `/audit/pending` returns `AuditCorrectionRow` rows (`correction_id`,
  `feedback{zone_id, reported_count, actual_count, ts_from, ts_to,
  failure_tags, notes, operator_id, wants_retrain}`, `outcome`,
  `reward_computed`, `created_at`) — not "predicted windows". The desk is
  submit → pending → **commit**; the reward lands at commit.
- The submit form posts the exact `CorrectionSubmit` body; the bad-delta
  guard (>25%) is frontend pedagogy, the hard gate is the commit 409.
- The heatmap is a pure-DOM zone × hour grid (deck.gl is reserved for
  venues that supply `floorplan.png`).
- `MiniVideoTile` sources its feed from `NEXT_PUBLIC_SHADO_HLS_URL`
  (`{camera}` placeholder, edge-node live HLS); `ZoneCard` renders one only
  when the venue config passes the camera binding. No feed key → explicit
  "withheld" note — never a silently-cycled still.
- Playwright E2E (`web/e2e`) boots the Next dev server and intercepts REST
  with the real backend JSON shapes: audit commit flow + 409 semantics,
  per-zone hourly → chart/heatmap/table, dashboard shell + dead-WS honest
  `reconnecting` chip. Screenshots are artifacts (only-on-failure + paths),
  not pixel assertions — font rendering differs across runner OSes.
- Zustand store slices must be selected as the Map ref and derived with
  `useMemo`; in-selector array spreads (`[...s.counts.values()]`) unlock a
  React 19 useSyncExternalStore render loop that only fires in a real
  browser (found by the dashboard E2E).
- E2E section 9 covers the harness reality: dev-server + REST interception
  (no backend needed), failure screenshots as artifacts, and the render-loop
  selector rule.
