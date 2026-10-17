# 15 — Build Workflow, Branching & Definition of Done

## 1. Two-branch contract

| Branch | Purpose | Rules |
|---|---|---|
| `main` | Production-ready documentation & code | Only gets content via merge from `test-debug` after all gates pass. No commits directly (except emergency docs fixes, which backported/merged back to test quickly). |
| `test-debug` | All building, testing, debugging | Everything new lands here first. CI full matrix required green before merge. |

Init sequence performed in this repo:

```
git init -b main
git add docs/ README.md && git commit -m "docs: complete build documentation set"
git switch -c test-debug
git push -u origin main test-debug        # after GitHub access is provided
```

## 2. Protocols used to build the system (in order)

Implementation order matches crates' dependency graph — built on `test-debug`:

```
Phase 0 (docs)   → this documentation set on main
Phase 1 (types)  → common-types: bounds, events, states, traits + 100% tests
Phase 2 (inputs) → ingestion-cctv (fixtures path first), ingestion-wifi (parsers/replay)
Phase 3 (brain)  → inference-engine: MockDetector→real OrtDetector, ByteTrack, crossing, fusion
Phase 4 (loops)  → rl-controller bandit (+ reward, watchdog, persistence)
Phase 5 (voice)  → vlm-orchestrator (morning diff, root-cause, COCO export) + mock provider
Phase 6 (ship)   → api-server routes, WS, webhooks, persistence, boot order
Phase 7 (eyes)   → dashboard live + audit + report
Phase 8 (steel)  → real hardware soak (L4), fine-tune feature flag, hot-swap, guardrail drills
```

Each phase: **write tests → implement → green on `test-debug` → merge `main`.** Never skip phases; phases can, however, run in sub-branches off `test-debug` (`td/<phase>-<topic>`), e.g. `td/7-dashboard-correctform`.

## 3. Commit conventions on test-debug

- Conventional Commits: `feat|fix|test|docs|perf|refactor(scope): …`
  - scopes: system crates `cctv|csi|infer|rl|vlm|api|dash|types|ops`.
- Every commit touching crates keeps CI green (no WIP commits directly; use `td/…` sub-branch with rebases/PRs if dirty).
- Bug fixes carry a `debug:` note in the body: symptom, tap evidence, root cause, regression test at line level.

## 4. CI (GitHub Actions) required checks for merge to main

| Check | Detail |
|---|---|
| `lint` | fmt, clippy `-D warnings -W clippy::unwrap_used`, pnpm lint (dash) |
| `unit` | cargo test workspace + proptest, insta |
| `integration` | `--features itest` incl sqlx tests, mock WS |
| `sim` | sim-replay day_normal + day_occlusion burn-down day test |
| `dash` | pnpm build, tsc --noEmit, Playwright suite on champ |
| `benches` | p95 budget × 1.05 gate (GPU runner; CPU variant relaxed documented) |
| `security` | cargo deny licenses+advisories, secret scan (gitleaks) |

## 5. Definition of Done for ANY merge to `main`

1. All CI checks green on the merge base (not just local).
2. No `.unwrap()` / `expect()` / `panic!` in library production paths (grep gate).
3. Every behavior change has ≥ 1 regression test pinned to the fixture corpus.
4. Perf budgets respected (or a written deviation note + issue to revisit).
5. `sim-replay` burn-down day passes with exact synthetic totals.
6. Docs updated in same PR for anything that changes a contract (`common-types`, API, SQL, dashboards).
7. Soak-worthy changes spent ≥ 24 h on the burn-in rig (docs/14 §9) without a crash or false count.
8. Merge message includes the "MERGED:" footer listing test gate results.

## 6. Nightly/fine-tune rollout policy

1. New weights always `staged` → `shadow` (docs/08 §6) 2 h at next morning boot on real floor.
2. Promote only if shadow divergence < 2% and no regression in canonical 5-frame self-test.
3. Any rollback: `model_registry.status='rollback'`, engine restores previous generation automatically.

## 7. Who works where (agent roles)

| Role | Branch |
|---|---|
| Rust engineer (interning + crates) | `test-debug` (+ sub-branches) |
| Dash engineer | `test-debug` + `td/dash-…` |
| Test/automation | `test-debug` |
| Release shepherd | merges `test-debug`→`main`, tags releases `vX.Y.Z`, publishes images |
| Everyone | reads `docs/06` before touching contracts; breaking contract change needs a review note (`docs-adr/YYYY-MM-DD.md`) |
