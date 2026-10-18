"use client";

import { useQuery } from "@tanstack/react-query";
import { apiFetch, useLive } from "@/lib/live";
import RlPanel from "@/components/RlPanel";
import type { RlState } from "@/lib/types";

/** RL knobs & runway visualization (docs/12 §2 /settings/policy). */
export default function PolicyPage() {
  const { data } = useQuery({ queryKey: ["rl-state"], queryFn: () => apiFetch<RlState>("/api/v1/rl/state"), retry: 1 });
  const rl = useLive((s) => s.rl);

  return (
    <div className="space-y-4" data-testid="policy-page">
      <h1 className="text-lg font-semibold text-zinc-100">RL policy</h1>
      <RlPanel transition={rl} />
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4 text-sm text-zinc-300">
        {data ? (
          <ul className="font-mono text-xs">
            <li>
              paused_until: <span data-testid="paused-until">{data.paused_until ?? "—"}</span>
            </li>
            <li>override_note: {data.override_note ?? "—"}</li>
            <li>
              rejected_24h: <span data-testid="rejected-24h">{data.rejected_24h}</span>
            </li>
          </ul>
        ) : (
          <p className="text-zinc-500">start the Rust server for live policy state</p>
        )}
        <p className="mt-3 text-xs text-zinc-500">
          Sliders are viewer-only; the bandit is the only automated actor and the
          browser never writes thresholds. Manual pauses go through the server API.
        </p>
      </section>
    </div>
  );
}
