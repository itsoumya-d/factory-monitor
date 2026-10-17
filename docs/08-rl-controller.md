# 08 — RL Controller Crate (`rl-controller`)

## 1. Design Philosophy

The RL agent is **not** allowed to touch counts, models, weights, or data. It only owns a tiny, safely-clamped action space that tunes ongoing detectors and fusion — a *contextual bandit with a deferred reward adapter*, upgradeable to PPO once reward shaping is validated for a factory month.

## 2. Action Space (hard-clamped)

| Dimension | Range | Effect |
|---|---|---|
| `τ_conf` | [0.25, 0.85] | detector confidence gate; the single most impactful knob |
| `τ_iou` | [0.30, 0.70] | ByteTrack association gate |
| `track_buffer` | [15, 90] frames @30tps = 0.5..3 s | zombie-track forgetting latency |
| `α_vision` | [0.05, 0.95] (β_wifi = 1-α) | fusion trust ratio |
| `fp_weight` (λ) | [0.5, 3.0] | multiplier of the FalsePositives term of reward |

## 3. State Encoding

`FactoryEnvState` → 12-d normalized vector (see `common-types`): `lux_norm, blur, csi_var, occlusion, scene_complexity, confτ_cur, iouτ_cur, α_cur, err_momentum_1h, err_momentum_24h, lifecycle_ohe(2)`. The `err_momenta` are exponential moving averages of yesterday's over/under-count signals.

## 4. Reward Function

$$R_t = -\Big(|N_{\text{pred}} - N_{\text{actual}}\Big| - \lambda \cdot \text{FalsePositives}\Big)$$

Two feedback routes into reward:

| Route | Trigger | Reward |
|---|---|---|
| **Immediate (intra-day)** | `PackageCountEvent` with `MODALITY_DIVERGENCE` or later reconciled by track-kill | small shaping term `±|Δw|` to discourage thrash; weight 0.1 of final reward |
| **Deferred (EOD audit)** | `HumanAuditFeedback` | full reward as above; **attributed** to actions over the feedback's time window viaeligibility traces (γ = 0.92, discrete steps = 30-min epochs) |

## 5. Policy Architecture

- **Base:** contextual bandit — linear contextualThompson sampling over action buckets. Apache reference: minimal, safe.
- **Upgrade (config flag)**: `burn` MLP: `12 → 64 → 32 → 5` (Policy softmax head + Value head), PPO clipped objective, lr 3e-4, γ 0.92, ε 0.2. Upgraded only after `test-debug` branch runs 2-week soak on bandit mode with real floors.
- Exploration: ε-greedy on bandit ε = 0.05 early morning only (**never** during peak throughput hours 13:00–17:00).

## 6. Runaway Guardrails (absolute, absolute, absolute)

1. `RlAction::clamped()` is the ONLY constructor; the `PolicyAgent::select_action` impl must route through it. Compilers may not silence; a `debug_assert!` also fires in dev.
2. **Stale-safety watchdog (implemented in `inference-engine`, not the RL crate** — cross-checking matter!):
   - If agent's action would move `τ_conf` outward of center more than ± 0.15 in one epoch → **reject that action**, keep last-safe, log `PolicyActionRejected`, fire webhook "RL action out of runway".
   - If 3 consecutive rejections → freeze policy, restore marble **last-known good snapshot weights**.
3. Manual overrides always beat the agent (dashboard "lock" switch sets `PolicyAgent::pause(until)`) — and the pause state is persisted across restarts.
4. New-model nights: retrained detector starts in **shadow mode** (compare outputs vs live for 2 h at 6–8 AM, don't act), only promoted if shadow divergence < 2%.

## 7. API (crate-level)

```rust
pub struct BanditPolicy { /* Thompson sampler, per-zone optional */ }

impl PolicyAgent for BanditPolicy {
    fn name(&self) -> &str { "bandit-v1" }
    async fn select_action(&mut self, state: &FactoryEnvState) -> RlAction { /* θ sample → weights → clamp */ }
    async fn update_policy(&mut self, fb: PolicyFeedback) -> Result<PolicyUpdateReport, anyhow::Error> { /* … */ }
}

pub struct PpoPolicy { /* burn module wrapper + replay window */ }  // gated behind `feature = "ppo"`
```

`PolicyUpdateReport { new_action, was_clamped, attribution_steps, next_exploration_window }` — the dashboard displays this to SREs.

## 8. Configuration & persistence

- Weights + buffers in burn `.berr`/bin SQL dumps under `data/rl/{zone_id}/{version}.rlw` + pg row `rl_audit` (docs/11).
- On cold start, restore latest-safe snapshot; if none → fully deterministic default action: `τ_conf=0.45, τ_iou=0.45, α=0.7, buffer=45`.
