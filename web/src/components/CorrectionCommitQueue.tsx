"use client";

import { useState } from "react";
import { apiFetch } from "@/lib/live";
import type { PendingCorrectionRow } from "@/lib/types";
import { FAILURE_TAGS } from "@/lib/formats";

/** Backend commit gate (routes.rs): deltas above this re-post with confirm. */
export const BIG_DELTA_CONFIRM = 100;

/** Decode the tag bit mask back into names (bit i ↔ FAILURE_TAGS[i]). */
function decodeFailureTags(mask: number): string[] {
  return FAILURE_TAGS.filter((_, bit) => (1 << bit) & mask);
}

/**
 * Pending COMMIT queue (docs/08 §3): corrections are submitted, then a second
 * operator commits them — only then does RL act. Deltas above 100 need an
 * explicit confirm checkbox or the backend answers 409 (routes.rs:412).
 */
export default function CorrectionCommitQueue({
  rows,
  onDone,
}: {
  rows: PendingCorrectionRow[];
  onDone: (correctionId: string) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmed, setConfirmed] = useState<Record<string, boolean>>({});

  const commit = async (row: PendingCorrectionRow, confirm: boolean) => {
    setBusy(row.correction_id);
    setError(null);
    try {
      await apiFetch(`/api/v1/audit/corrections/${encodeURIComponent(row.correction_id)}/commit`, {
        method: "POST",
        body: JSON.stringify({ confirm }),
      });
      onDone(row.correction_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  if (rows.length === 0) return null;

  return (
    <section className="rounded border border-zinc-800 bg-zinc-900 p-4" data-testid="commit-queue">
      <h3 className="text-sm font-semibold text-zinc-200">Pending commits</h3>
      <p className="text-xs text-zinc-400">
        submitted corrections awaiting a commit — the reward lands when you
        commit, not when you submit
      </p>
      <ul className="mt-2 space-y-2" data-testid="commit-list">
        {rows.map((row) => {
          const f = row.feedback;
          const delta = Math.abs(f.actual_count - f.reported_count);
          const big = delta > BIG_DELTA_CONFIRM;
          const id = row.correction_id;
          return (
            <li key={id} className="rounded border border-zinc-800 bg-zinc-950 p-2" data-testid={`commit-row-${id}`}>
              <p className="font-mono text-xs text-zinc-300">
                {f.zone_id} · {f.ts_from.slice(11, 16)}→{f.ts_to.slice(11, 16)} ·{" "}
                {f.reported_count}→{f.actual_count} ·{f.failure_tags}&nbsp;
                {decodeFailureTags(f.failure_tags).join(" | ") || "no tags"} ·
                reward {row.reward_computed != null ? row.reward_computed.toFixed(2) : "pending"} ·
                by {f.operator_id}
              </p>
              {big && (
                <label className="mt-1 flex items-center gap-2 text-xs text-amber-400">
                  <input
                    type="checkbox"
                    data-testid={`commit-confirm-${id}`}
                    checked={Boolean(confirmed[id])}
                    onChange={(e) => setConfirmed((c) => ({ ...c, [id]: e.target.checked }))}
                  />
                  delta {delta} &gt; {BIG_DELTA_CONFIRM} — confirm to commit
                </label>
              )}
              <button
                type="button"
                data-testid={`commit-btn-${id}`}
                disabled={busy === id || (big && !confirmed[id])}
                onClick={() => void commit(row, big ? Boolean(confirmed[id]) : true)}
                className="mt-1 rounded bg-lime-500 px-3 py-1 text-xs font-semibold text-zinc-950 disabled:opacity-40"
              >
                {busy === id ? "committing…" : "commit"}
              </button>
            </li>
          );
        })}
      </ul>
      {error && <p className="mt-2 text-xs text-red-400">{error}</p>}
    </section>
  );
}
