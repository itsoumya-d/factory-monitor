# Branch: `test-debug` — Test & Debug Harness

This branch is where **everything is built, tested, and debugged before it merges to `main`** (see `../docs/15-build-workflow.md`).

## What differs here

| Item | This branch |
|---|---|
| Purpose | All implementation + full test matrix + debug tooling |
| Merge flow | `test-debug` → `main` **only** when Definition of Done (`docs/15 §5`) is green |
| Debug taps | `--tap` frame/CSI/decision dumps enabled (never enabled in prod builds) |
| Features enabled | `itest`, `sim`, `bench`, `debug-taps` (later: `fine_tune` during Phase-8 drills) |

## Current state

- Phase 0 (docs) complete and merged into `main` (91b4582).
- Next: Phase 1 — `crates/common-types` (bounds, events, states, traits) with 100% test coverage per `docs/06`.

## Per-change checklist (copy into your PR/description)

```markdown
- [ ] L1 unit: cargo test (module …) — N tests green (+proptest where bounds/math)
- [ ] L2 integration: cargo test --features itest
- [ ] L3 sim: sim-replay fixtures/day_*.json — ground-truth totals exact
- [ ] Lint: clippy -D warnings -W clippy::unwrap_used; fmt --check
- [ ] Perf: criterion budgets ok (only if perf-path code touched)
- [ ] Coverage: line coverage ≥85% on modules touched
- [ ] No .unwrap()/expect()/panic! in production paths
- [ ] Docs updated if a contract changed (types/API/SQL/UI)
- [ ] Soak: 24 h burn-in rig green (soak-worthy changes only)
```

## Debug quick recipes

```bash
# replay a synthetic fixture day at 4x speed, dump pipeline taps
RUST_LOG=debug cargo run -p shado-engine -- replay fixtures/day_occlusion.json --speed 4x --tap out/

# env sanity for support
cargo run -p shado-doctor

# golden self-test of the detector against canonical frames
curl -X POST localhost:8080/api/v1/admin/models/selftest
```

## Fixture corpus location (to be committed as Phase 2+ lands)

```
fixtures/
├── day_normal.json
├── day_occlusion.json
├── day_lighting.json
├── csi_walk.npy / csi_idle.npy / csi_forklift.npy
├── frames_line2_seq.h264        # gitignored: too heavy; downloaded via scripts/fetch_fixtures.sh
└── corrections_sample.jsonl
```

Binary/large fixtures are excluded from git (see `.gitignore`) and pulled via `scripts/fetch_fixtures.sh` when needed; their **checksum hashes are committed** in `fixtures/MANIFEST.sha256` and verified by the test harness.
