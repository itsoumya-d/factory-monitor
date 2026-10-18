"use client";

import { useState } from "react";
import { parseUncertainty } from "@/lib/formats";
import { apiFetch } from "@/lib/live";
import BoxEditor, { CATEGORIES } from "@/components/BoxEditor";
import { cocoApproveBody, imageUrl, packageCoco, type CocoDataset, type DraftBox } from "@/lib/coco";

export interface LabelRow {
  frame_id: string;
  zone_id: string;
  ts: string;
  camera_id: string;
  divergence: number | null;
  uncertainty_mask: number;
  promotion: "Amber" | "Uploaded" | "AutoLabeled" | "LabeledOk" | "Trained";
  coco_annotations: CocoDataset | null;
}

/** VLM-side boxes (annotation 0-based i → taxonomy category_id i+1). */
function boxesFromCoco(coco: CocoDataset | null): DraftBox[] {
  if (!coco) return [];
  const w = coco.images[0]?.width ?? 1;
  const h = coco.images[0]?.height ?? 1;
  return coco.annotations.map((a) => ({
    x: a.bbox[0] / w,
    y: a.bbox[1] / h,
    w: a.bbox[2] / w,
    h: a.bbox[3] / h,
    category: coco.categories[a.category_id - 1] ?? CATEGORIES[0],
  }));
}

/** Rejects must carry a reason (backend 400 otherwise; audit trail). */
const REJECT_REASON = "operator determined the frame is not labelable";

/**
 * One queued frame: inspect (frame bytes + drag-to-fix) → approve POST
 * (teach = approve with annotation fixes), or reject POST with reason.
 */
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
  const [inspecting, setInspecting] = useState(false);
  const [boxes, setBoxes] = useState<DraftBox[]>(() => boxesFromCoco(row.coco_annotations));
  const [imageError, setImageError] = useState<string | null>(null);

  const approve = async () => {
    if (!operator.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const coco =
        boxes.length > 0
          ? packageCoco({
              frameId: row.frame_id,
              fileName: `${row.frame_id}.jpg`,
              width: 640,
              height: 480,
              boxes,
              categories: [...CATEGORIES],
            })
          : null;
      await apiFetch(`/api/v1/labeling/${encodeURIComponent(row.frame_id)}/approve`, {
        method: "POST",
        body: JSON.stringify(cocoApproveBody(operator.trim(), coco)),
      });
      onDone(row.frame_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const loadAndOpen = () => setInspecting(true);

  const reject = async () => {
    if (!operator.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await apiFetch(`/api/v1/labeling/${encodeURIComponent(row.frame_id)}/reject`, {
        method: "POST",
        body: JSON.stringify({ operator_id: operator.trim(), reason: REJECT_REASON }),
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
      <button
        type="button"
        data-testid={`inspect-${row.frame_id}`}
        onClick={loadAndOpen}
        className="mt-1 rounded border border-zinc-700 px-2 py-0.5 text-[11px] text-zinc-300"
      >
        {inspecting ? "fixing boxes" : "inspect"}
      </button>
      {inspecting && (
        <div className="mt-2">
          {/* eslint-disable-next-line @next/next/no-img-element -- operator-served
              lowres bytes come from the Rust image route, not a CDN. */}
          <img
            src={imageUrl(row.frame_id)}
            alt={`frame ${row.frame_id} lowres`}
            width={320}
            height={240}
            className="rounded bg-zinc-950"
            onError={() => setImageError(imageUrl(row.frame_id))}
          />
          {imageError && (
            <p className="text-[11px] text-red-400" data-testid={`img-error-${row.frame_id}`}>
              frame bytes offline ({imageError})
            </p>
          )}
          <BoxEditor categoryIndex={0} boxes={boxes} onChange={setBoxes} />
        </div>
      )}
      {error && <p className="text-sm text-red-400">{error}</p>}
      <div className="mt-2 flex gap-2">
        <button
          type="button"
          onClick={() => void approve()}
          disabled={busy || operator.trim().length === 0}
          className="rounded bg-lime-500 px-3 py-1 text-sm font-medium text-zinc-950 disabled:opacity-40"
          data-testid={`approve-${row.frame_id}`}
        >
          {busy ? "sending…" : "approve"}
        </button>
        <button
          type="button"
          onClick={() => void reject()}
          disabled={busy || operator.trim().length === 0}
          className="rounded bg-zinc-800 px-3 py-1 text-sm text-zinc-300 disabled:opacity-40"
          data-testid={`reject-${row.frame_id}`}
        >
          reject
        </button>
      </div>
    </li>
  );
}
