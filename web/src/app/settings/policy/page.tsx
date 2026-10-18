"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiFetch, useLive } from "@/lib/live";
import RlPanel from "@/components/RlPanel";
import type { RlState, RlHistoryRow } from "@/lib/types";

const PAUSE_PRESETS = [
  { label: "30 min", ms: 30 * 60_000 },
  { label: "2 hours", ms: 2 * 3_600_000 },
  { label: "8 hours", ms: 8 * 3_600_000 },
] as const;

/** RL knobs & runway visualization (docs/12 §2 /settings/policy). */
export default function PolicyPage() {
  const [busy, setBusy] = useState<"pause" | "revert" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const { data } = useQuery({
    queryKey: ["rl-state"],
    queryFn: () => apiFetch<RlState>("/api/v1/rl/state"),
    retry: 1,
  });
  const rl = useLive((s) => s.rl);

  const pausedUntil = data?.paused_until ?? null;
  const paused = pausedUntil ? new Date(pausedUntil).getTime() > Date.now() : false;

  const pause = async (ms: number) => {
    setBusy("pause");
    setErr(null);
    try {
      await apiFetch("/api/v1/rl/override", {
        method: "POST",
        body: JSON.stringify({
          until: new Date(Date.now() + ms).toISOString(),
          note: `manual pause (${PAUSE_PRESETS.find((p) => p.ms === ms)?.label})`,
        }),
      });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const revert = async () => {
    setBusy("revert");
    setErr(null);
    try {
      await apiFetch("/api/v1/rl/override", {
        method: "POST",
        body: JSON.stringify({
          until: new Date(Date.now() + 1_000).toISOString(),
          note: "revert to last-safe (bandit defaults, clamps rearmed)",
        }),
      });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const today = new Date().toISOString().slice(0, 10);
  const historyQ = useQuery({
    queryKey: ["rl-history", today],
    queryFn: () =>
      apiFetch<{ date: string; transitions: RlHistoryRow[] }>(
        `/api/v1/rl/history?date=${today}`,
      ),
    retry: 1,
  });

  return (
    <div className="space-y-4" data-testid="policy-page">
      <h1 className="text-lg font-semibold text-zinc-100">RL policy</h1>
      <RlPanel transition={rl} />
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4 text-sm text-zinc-300">
        {data ? (
          <ul className="font-mono text-xs">
            <li>
              paused_until: <span data-testid="paused-until">{pausedUntil ?? "—"}</span>
            </li>
            <li>
              override_note: <span data-testid="override-note">{data.override_note ?? "—"}</span>
            </li>
            <li>
              rejected_24h: <span data-testid="rejected-24h">{data.rejected_24h}</span>
            </li>
          </ul>
        ) : (
          <p className="text-zinc-500">start the Rust server for live policy state</p>
        )}
        <div className="mt-3 flex flex-wrap gap-2" data-testid="policy-actions">
          {PAUSE_PRESETS.map((p) => (
            <button
              key={p.label}
              type="button"
              onClick={() => void pause(p.ms)}
              disabled={busy !== null}
              className="rounded bg-amber-600 px-3 py-1.5 text-xs font-medium text-zinc-950 disabled:opacity-40"
              data-testid={`pause-${p.label.replace(/[^a-z0-9]/gi, "-")}`}
            >
              {busy === "pause" ? "pausing…" : `pause ${p.label}`}
            </button>
          ))}
          <button
            type="button"
            onClick={() => void revert()}
            disabled={busy !== null}
            className="rounded bg-lime-500 px-3 py-1.5 text-xs font-medium text-zinc-950 disabled:opacity-40"
            data-testid="revert-button"
          >
            {busy === "revert" ? "reverting…" : "revert to last-safe"}
          </button>
          {paused && (
            <span
              data-testid="paused-chip"
              className="rounded bg-amber-500/20 px-2 py-0.5 font-mono text-xs text-amber-300"
            >
              policy paused until {String(pausedUntil).slice(11, 19)}
            </span>
          )}
        </div>
        {err && (
          <p className="mt-2 text-xs text-red-400" data-testid="policy-error">
            {err}
          </p>
        )}
        <p className="mt-3 text-xs text-zinc-500">
          Sliders are viewer-only; the bandit is the only automated actor and the
          browser never writes thresholds. Manual pauses go through the server API
          (POST /api/v1/rl/override); revert re-arms the guardrail clamps.
        </p>
      </section>
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h3 className="text-sm font-semibold text-zinc-200">rl_audit · {today}</h3>
        {historyQ.isLoading ? (
          <p className="text-sm text-zinc-500">loading history…</p>
        ) : historyQ.isError ? (
          <p className="text-sm text-red-400">history unavailable</p>
        ) : (historyQ.data?.transitions?.length ?? 0) === 0 ? (
          <p className="text-sm text-zinc-500" data-testid="rl-history-empty">
            no transitions today
          </p>
        ) : (
          <table className="mt-2 w-full font-mono text-xs text-zinc-300">
            <tbody data-testid="rl-history">
              {historyQ.data?.transitions.map((t) => (
                <tr key={t.transition_id} className="border-t border-zinc-800">
                  <td className="py-1 pr-2">{t.ts.slice(11, 19)}</td>
                  <td className="py-1 pr-2">
                    <span
                      className={
                        t.rejected ? "text-red-400" : t.was_clamped ? "text-amber-300" : "text-zinc-300"
                      }
                    >
                      {t.reason}
                      {t.rejected ? " (rejected)" : t.was_clamped ? " (clamped)" : ""}
                    </span>
                  </td>
                  <td className="py-1 pr-2 text-right">
                    {"τ"} {t.before.conf_threshold.toFixed(2)} {"→"} {t.after.conf_threshold.toFixed(2)}
                  </td>
                  <td className="py-1 text-right">{t.reward?.toFixed(3) ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
