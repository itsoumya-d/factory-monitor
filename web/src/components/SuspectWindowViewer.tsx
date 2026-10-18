"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import { imageUrl } from "@/lib/coco";
import { parseUncertainty } from "@/lib/formats";
import type { SuspectWindow as WindowRow, SuspectFrame } from "@/lib/types";

/**
 * docs/12 §3 SuspectWindowViewer: sampled ±span context frames around one
 * suspect moment (backend GET /api/v1/snapshots/window), tiles scrub into a
 * big preview with the uncertainty flags of the selected frame.
 */
export default function SuspectWindowViewer({
  zoneId,
  ts,
  onClose,
}: {
  zoneId: string;
  ts: string;
  onClose: () => void;
}) {
  const [selected, setSelected] = useState<SuspectFrame | null>(null);
  const [imgErrors, setImgErrors] = useState<Record<string, boolean>>({});
  const q = useQuery({
    queryKey: ["suspect-window", zoneId, ts],
    queryFn: () =>
      apiFetch<WindowRow>(
        `/api/v1/snapshots/window?zone_id=${encodeURIComponent(zoneId)}&ts=${encodeURIComponent(ts)}&span=1800`,
      ),
    retry: 1,
  });

  const win = q.data;
  const frames = win?.frames ?? [];
  const active = selected ?? frames.find((f) => f.frame_id === win?.focal_frame_id) ?? null;
  const flags = active ? parseUncertainty(active.uncertainty_mask) : [];

  if (q.isLoading) {
    return (
      <div className="rounded border border-zinc-800 bg-zinc-900 p-4 text-sm text-zinc-300" data-testid="suspect-window">
        <p className="text-zinc-500">loading context frames…</p>
      </div>
    );
  }

  if (q.isError) {
    return (
      <div className="rounded border border-red-900 bg-zinc-900 p-4 text-sm text-red-400" data-testid="suspect-window">
        <p>context frames unavailable: {q.error instanceof Error ? q.error.message : String(q.error)}</p>
        <button
          type="button"
          onClick={onClose}
          className="mt-2 rounded bg-zinc-800 px-2 py-1 text-zinc-300"
          data-testid="suspect-close"
        >
          close
        </button>
      </div>
    );
  }

  if (frames.length === 0) {
    return (
      <div className="rounded border border-zinc-800 bg-zinc-900 p-4 text-sm text-zinc-400" data-testid="suspect-window">
        <p data-testid="suspect-empty">
          no sampled frames near {zoneId} at {ts} — window is empty at these edges
        </p>
        <button
          type="button"
          onClick={onClose}
          className="mt-2 rounded bg-zinc-800 px-2 py-1 text-zinc-300"
          data-testid="suspect-close"
        >
          close
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-2 rounded border border-zinc-800 bg-zinc-900 p-4" data-testid="suspect-window">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-semibold text-zinc-200">
          window context · {zoneId} · around {ts.slice(11, 19)}
        </h4>
        <button
          type="button"
          onClick={onClose}
          className="rounded bg-zinc-800 px-2 py-1 text-xs text-zinc-300"
          data-testid="suspect-close"
        >
          close
        </button>
      </div>

      <div className="flex gap-2 overflow-x-auto" data-testid="suspect-strip">
        {frames.map((f, i) => (
          <button
            key={f.frame_id}
            type="button"
            onClick={() => setSelected(f)}
            aria-label={`frame at ${f.ts}`}
            data-testid={`suspect-tile-${i}`}
            className={`shrink-0 rounded border p-1 ${
              active?.frame_id === f.frame_id ? "border-lime-500" : "border-zinc-700"
            }`}
          >
            {imgErrors[f.frame_id] ? (
              <span className="block h-12 w-16 bg-zinc-800 text-center text-[10px] text-zinc-500" data-testid={`img-error-${i}`}>
                bytes missing
              </span>
            ) : (
              /* eslint-disable-next-line @next/next/no-img-element */
              <img
                src={imageUrl(f.frame_id)}
                alt=""
                width={64}
                height={48}
                className="block h-12 w-16 object-cover"
                onError={(e) => {
                  const el = e.currentTarget;
                  setImgErrors((m) => {
                    const next = { ...m };
                    next[f.frame_id] = true;
                    return next;
                  });
                  void el;
                }}
              />
            )}
            <span className="block text-[10px] font-mono text-zinc-500">{f.ts.slice(11, 19)}</span>
          </button>
        ))}
      </div>

      {active && (
        <div className="flex items-start gap-3" data-testid="suspect-preview">
          <div className="rounded border border-zinc-700 p-1">
            {imgErrors[active.frame_id] ? (
              <span className="block h-24 w-40 bg-zinc-800 text-center text-[10px] leading-24 text-zinc-500">
                bytes missing
              </span>
            ) : (
              /* eslint-disable-next-line @next/next/no-img-element */
              <img
                src={imageUrl(active.frame_id)}
                alt=""
                width={160}
                height={96}
                className="block h-24 w-40 object-cover"
                onError={() => {
                  setImgErrors((m) => ({ ...m, [active.frame_id]: true }));
                }}
              />
            )}
          </div>
          <ul className="space-y-0.5 font-mono text-xs text-zinc-400">
            <li data-testid="suspect-ts">{active.ts}</li>
            <li>camera {active.camera_id}</li>
            <li>promotion {active.promotion}</li>
            <li>divergence {active.divergence ?? "—"}</li>
            {active.vlm_root_cause && (
              <li data-testid="suspect-root-cause" className="text-amber-300">
                root cause: {active.vlm_root_cause}
              </li>
            )}
            <li className="flex gap-1" data-testid="suspect-flags">
              {flags.length > 0 ? (
                flags.map((fl) => (
                  <span key={fl} className="rounded bg-amber-500/20 px-1 text-amber-300">
                    {fl}
                  </span>
                ))
              ) : (
                <span className="text-zinc-500">no uncertainty flags</span>
              )}
            </li>
          </ul>
        </div>
      )}
    </div>
  );
}
