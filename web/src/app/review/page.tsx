"use client";

import { useState } from "react";
import { useQueryClient, useQuery } from "@tanstack/react-query";
import LabelApprover, { type LabelRow } from "@/components/LabelApprover";
import SuspectWindowViewer from "@/components/SuspectWindowViewer";
import { apiFetch, useLive } from "@/lib/live";

export default function ReviewPage() {
  const [operator, setOperator] = useState("");
  const [windowCtx, setWindowCtx] = useState<{ zone: string; ts: string } | null>(null);
  const queryClient = useQueryClient();
  const labelCount = useLive((s) => s.labels.length);

  const queueQ = useQuery({
    queryKey: ["label-queue", labelCount],
    queryFn: () => apiFetch<{ queue: LabelRow[] }>("/api/v1/labeling/queue"),
    refetchInterval: 30_000,
  });

  const remove = (frameId: string) => {
    queryClient.setQueryData<{ queue: LabelRow[] }>(["label-queue", 0], (prev) =>
      prev ? { queue: prev.queue.filter((r) => r.frame_id !== frameId) } : prev,
    );
    void queryClient.invalidateQueries({ queryKey: ["label-queue"] });
  };

  const rows = queueQ.data?.queue ?? [];
  return (
    <div className="space-y-3">
      <h1 className="text-lg font-semibold">Review queue</h1>
      <p className="text-sm text-zinc-400">
        amber snapshots awaiting a human eye. Approving marks the frame
        LabeledOk so the auto-labeler leaves it alone.
      </p>
      <label className="block text-sm text-zinc-300">
        operator id
        <input
          value={operator}
          onChange={(e) => setOperator(e.target.value)}
          data-testid="operator-input"
          className="ml-2 w-44 rounded border border-zinc-700 bg-zinc-950 px-2 py-0.5 font-mono"
          placeholder="op_1"
        />
      </label>
      <button
        type="button"
        onClick={() => void queryClient.invalidateQueries({ queryKey: ["label-queue"] })}
        className="rounded border border-zinc-700 px-3 py-1 text-sm text-zinc-300"
        data-testid="reload-queue"
      >
        reload
      </button>
      {queueQ.error && (
        <p className="text-sm text-red-400">{String(queueQ.error.message)}</p>
      )}
      <ul className="space-y-2" data-testid="review-list">
        {queueQ.isLoading ? (
          <li className="text-sm text-zinc-500">loading queue…</li>
        ) : rows.length === 0 ? (
          <li className="text-sm text-zinc-500">queue clear — nothing flagged</li>
        ) : (
          rows.map((row) => (
            <li key={row.frame_id} className="space-y-1">
              <LabelApprover row={row} operator={operator} onDone={remove} />
              <button
                type="button"
                onClick={() => setWindowCtx({ zone: row.zone_id, ts: row.ts })}
                className="rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-300"
                data-testid={`window-${row.frame_id}`}
              >
                window ±30m
              </button>
            </li>
          ))
        )}
      </ul>
      {windowCtx && (
        <SuspectWindowViewer
          zoneId={windowCtx.zone}
          ts={windowCtx.ts}
          onClose={() => setWindowCtx(null)}
        />
      )}
    </div>
  );
}
