"use client";

import type { RlTransition } from "@/lib/types";

const RUNWAYS = {
  conf_threshold: [0.25, 0.85],
  iou_threshold: [0.25, 0.7],
  vision_weight: [0.05, 0.95],
  track_buffer_frames: [20, 90],
} as const;

/**
 * RL knobs (docs/12 §3 RlPanel): last transition + runway visualization.
 * Sliders are disabled: the browser is a viewer; manual overrides go
 * through /api/v1/rl/override (Pause policy switch lives on the server).
 */
export default function RlPanel({ transition }: { transition: RlTransition | null }) {
  const pick: Record<string, number> = (transition?.after ?? {
    conf_threshold: 0.45,
    iou_threshold: 0.45,
    vision_weight: 0.7,
    track_buffer_frames: 45,
  }) as Record<string, number>;

  return (
    <section data-testid="rl-panel" className="space-y-3 rounded border border-zinc-800 bg-zinc-900 p-4">
      <h3 className="text-sm font-semibold text-zinc-200">RL policy (bandit-v1)</h3>
      <ul className="space-y-2 font-mono text-xs text-zinc-300">
        {Object.entries(RUNWAYS).map(([key, [lo, hi]]) => {
          const v = pick[key] ?? lo;
          const pct = ((v - lo) / (hi - lo)) * 100;
          return (
            <li key={key}>
              <div className="flex justify-between text-zinc-400">
                <label htmlFor={`rl-${key}`}>{key}</label>
                <span data-testid={`rl-current-${key}`}>{v}</span>
              </div>
              <input
                id={`rl-${key}`}
                type="range"
                disabled
                min={lo}
                max={hi}
                step={(hi - lo) / 100}
                value={v}
                onChange={() => undefined}
                className="w-full accent-lime-400"
              />
              <div className="h-1 rounded bg-lime-400/70" style={{ width: `${pct}%` }} />
              <div className="text-[10px] text-zinc-500">
                runway {lo}–{hi}
              </div>
            </li>
          );
        })}
      </ul>
      {transition && (
        <p className="text-xs text-zinc-400" data-testid="rl-reason">
          {transition.rejected ? "rejected: " : "reason: "}
          {transition.reason}
        </p>
      )}
    </section>
  );
}
