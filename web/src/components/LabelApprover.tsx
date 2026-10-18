"use client";

import { useState } from "react";
import { parseUncertainty } from "@/lib/formats";
import { apiFetch } from "@/lib/live";

export interface LabelRow {
  frame_id: string;
  zone_id: string;
  ts: string;
  camera_id: string;
  divergence: number | null;
  uncertainty_mask: number;
  promotion: "Amber" | "Uploaded" | "AutoLabeled" | "LabeledOk" | "Trained";
}

/** One queued frame: uncertainty chips + approve POST (docs/09 §3). */
export default function LabelApprover({
  row,
  operator,
  onDone,
}: {
  row: LabelRow;
  operator: string;
  onDone: (frameId: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const approve = async () => {
    if (!operator.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await apiFetch(`/api/v1/labeling/${encodeURIComponent(row.frame_id)}/approve`, {
        method: "POST",
        body: JSON.stringify({ operator_id: operator.trim() }),
      });
      onDone(row.frame_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const flags = parseUncertainty(row.uncertainty_mask);
  return (
    <li className="rounded border border-zinc-800 bg-zinc-900 p-3" data-testid={`label-${row.frame_id}`}>
      <p className="font-mono text-xs text-zinc-300">
        {row.zone_id} · {row.camera_id} · {row.ts.slice(11, 19)} ·{" "}
        <span className="text-amber-400">{row.promotion.toLowerCase()}</span>
      </p>
      <p className="text-sm text-zinc-400">
        divergence {row.divergence != null ? row.divergence.toFixed(2) : "n/a"} ·{" "}
        {flags.length > 0 ? (
          <span className="text-zinc-300">{flags.join(" | ")}</span>
        ) : (
          "clean capture"
        )}
      </p>
      {error && <p className="text-sm text-red-400">{error}</p>}
      <button
        type="button"
        onClick={() => void approve()}
        disabled={busy || operator.trim().length === 0}
        className="mt-2 rounded bg-lime-500 px-3 py-1 text-sm font-medium text-zinc-950 disabled:opacity-40"
        data-testid={`approve-${row.frame_id}`}
      >
        {busy ? "approving…" : "approve"}
      </button>
    </li>
  );
}
