"use client";

import { useState } from "react";
import {
  type GeoPoint,
  type RoiLine,
  distPointToSegmentSq,
  roiSubmitBody,
  validateRoiLines,
} from "@/lib/roi";
import { apiFetch } from "@/lib/live";

/** Canvas square for the normalized [0,1] image space (docs/12 §3). */
export const CANVAS_PX = 320;
/** Grip radius, in normalized units. */
export const CLICK_RADIUS = 12 / CANVAS_PX;

export interface ShapeResult {
  zone_id: string;
  applied: boolean;
  lines: number;
}

export interface RoiEditorProps {
  zones: string[];
}

/**
 * Drag new counting lines: click-to-place-rope points, "End line" finishes
 * the active rope; clicking near a finished rope's last point pops it
 * (undo-step). Only legal lines submit ({"lines":[[[x,y],...]]} body).
 */
export default function RoiEditor({ zones }: RoiEditorProps) {
  const [zone, setZone] = useState(zones[0] ?? "");
  const [lines, setLines] = useState<RoiLine[]>([{ points: [] }]);
  const [applied, setApplied] = useState<ShapeResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const nonEmpty = lines.filter((l) => l.points.length >= 2);
  const activeIndex = lines.findIndex((l) => l.points.length < 2);
  const invalidReason = validateRoiLines(nonEmpty);

  const clickPos = (e: React.MouseEvent<HTMLDivElement>): GeoPoint => {
    // offsetX/OffsetY are canvas-pixel-local (getBoundingClientRect is zero
    // in jsdom recharts-less layouts); normalized on the square canvas.
    return { x: e.nativeEvent.offsetX / CANVAS_PX, y: e.nativeEvent.offsetY / CANVAS_PX };
  };

  const findGripNear = (p: GeoPoint): { line: number; at: number } | null => {
    for (let i = 0; i < lines.length; i++) {
      const pts = lines[i].points;
      for (let j = pts.length - 1; j >= 0; j--) {
        const a = pts[j];
        const b = pts[j - 1] ?? pts[j];
        if (distPointToSegmentSq(p, a, b) <= CLICK_RADIUS * CLICK_RADIUS) {
          return { line: i, at: j };
        }
      }
    }
    return null;
  };

  const onCanvasClick = (e: React.MouseEvent<HTMLDivElement>) => {
    setErr(null);
    setApplied(null);
    const p = clickPos(e);
    const grip = findGripNear(p);
    if (grip !== null && lines[grip.line].points.length >= 2) {
      setLines((current) =>
        current.map((l, i) => (i === grip.line ? { points: l.points.slice(0, -1) } : l)),
      );
      return;
    }
    setLines((current) => {
      const active =
        current[0]?.points.length === 0
          ? 0
          : current.findIndex((l) => l.points.length < 2);
      if (active < 0) return [...current, { points: [p] }];
      return current.map((l, i) => (i === active ? { points: [...l.points, p] } : l));
    });
  };

  const endLine = () =>
    setLines((current) => {
      const lastUnfinished = current[current.length - 1] ?? { points: [] };
      return lastUnfinished.points.length >= 2
        ? [...current, { points: [] }]
        : current;
    });

  async function submit() {
    try {
      const body = roiSubmitBody(nonEmpty);
      const res = await apiFetch<ShapeResult>(
        `/api/v1/calibration/rois/${encodeURIComponent(zone)}`,
        { method: "POST", body: JSON.stringify(body) },
      );
      setApplied(res);
      setErr(null);
    } catch (e) {
      setErr(String(e).replace(/^Error:\s*/, ""));
    }
  }

  const pathOf = (l: RoiLine): string =>
    l.points
      .map((p) => `${(p.x * CANVAS_PX).toFixed(1)},${(p.y * CANVAS_PX).toFixed(1)}`)
      .join(" ");

  return (
    <div className="flex gap-6" data-testid="roi-editor">
      <div
        data-testid="roi-canvas"
        role="img"
        aria-label={zone ? `floor plan for ${zone}` : "floor plan"}
        onClick={onCanvasClick}
        style={{ width: CANVAS_PX, height: CANVAS_PX }}
        className="relative cursor-crosshair rounded border-2 border-zinc-700 bg-zinc-900"
      >
        <svg viewBox={`0 0 ${CANVAS_PX} ${CANVAS_PX}`} className="absolute inset-0">
          {lines.map((l, i) => (
            <polyline
              key={i}
              points={pathOf(l)}
              fill="none"
              stroke={i === activeIndex ? "#a3e635" : "#67e8f9"}
              strokeWidth={4}
              data-testid={`roi-line-${i}`}
            />
          ))}
          {lines.flatMap((l, i) =>
            l.points.map((p, j) => (
              <circle
                key={`${i}-${j}`}
                cx={p.x * CANVAS_PX}
                cy={p.y * CANVAS_PX}
                r={6}
                fill={i === activeIndex ? "#a3e635" : "#67e8f9"}
                data-testid={`roi-handle-${i}-${j}`}
              />
            )),
          )}
        </svg>
      </div>

      <div className="w-64 space-y-3">
        <label htmlFor="roi-zone" className="block text-xs text-zinc-400">
          Zone
        </label>
        <select
          id="roi-zone"
          data-testid="roi-zone-select"
          className="w-full rounded bg-zinc-800 px-2 py-1.5 text-zinc-100"
          value={zone}
          onChange={(e) => setZone(e.target.value)}
        >
          <option value="" />
          {zones.map((z) => (
            <option key={z} value={z}>
              {z}
            </option>
          ))}
        </select>

        <button
          type="button"
          data-testid="roi-end-line"
          onClick={endLine}
          className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm text-zinc-200"
        >
          End line &amp; start next
        </button>

        <p data-testid="roi-status" className="font-mono text-xs text-zinc-400">
          {nonEmpty.length} finished · {lines[activeIndex]?.points.length ?? 0} pending
          {invalidReason ? ` · ${invalidReason}` : ""}
        </p>

        <button
          type="button"
          data-testid="roi-submit"
          onClick={() => void submit()}
          disabled={!zone || nonEmpty.length === 0 || invalidReason !== null}
          className="w-full rounded bg-lime-500 px-3 py-2 text-sm font-semibold text-zinc-950 disabled:opacity-40"
        >
          Apply ROI lines
        </button>

        {applied && (
          <p data-testid="roi-applied" className="font-mono text-xs text-lime-300">
            {applied.zone_id}: applied {applied.lines}
          </p>
        )}
        {err && <p className="text-xs text-red-400">{err}</p>}
      </div>
    </div>
  );
}
