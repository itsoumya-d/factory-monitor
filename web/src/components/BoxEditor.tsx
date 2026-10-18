"use client";

import { useState } from "react";
import type { DraftBox } from "@/lib/coco";

/**
 * Drag-to-fix boxes (docs/12 §3 LabelApprover "Approve/fix/Teach"): SVG
 * overlay on the label-viewer frame. Coordinates are normalized [0..1]
 * of the nominal lowres frame; packageCoco turns them into pixel bboxes.
 *
 * The editor logical size is fixed (320×240) like the ROI canvas so both
 * tests and the browser see the same pixel space.
 */
export const EDITOR_W = 320;
export const EDITOR_H = 240;

/** 1-indexed parallel of the Rust test taxonomy (auto_label.rs:142). */
export const CATEGORIES = ["package", "pallet"] as const;


const CATEGORY_GLYPH: Record<string, string> = {
  package: "#67e8f9",
  pallet: "#a3e635",
};
const stroke = (i: number, category: string): string =>
  CATEGORY_GLYPH[category] ?? ["#67e8f9", "#a3e635", "#fbbf24"][i % 3];

interface Props {
  categoryIndex: number;
  boxes: DraftBox[];
  onChange: (boxes: DraftBox[]) => void;
}

export default function BoxEditor({ categoryIndex, boxes, onChange }: Props) {
  const [draft, setDraft] = useState<{ x: number; y: number } | null>(null);
  const [selected, setSelected] = useState<number | null>(null);

  const pos = (raw: { offsetX: number; offsetY: number }) => ({
    x: Math.min(1, Math.max(0, raw.offsetX / EDITOR_W)),
    y: Math.min(1, Math.max(0, raw.offsetY / EDITOR_H)),
  });

  const onDown = (e: React.PointerEvent<SVGSVGElement>) => {
    setSelected(null);
    setDraft(pos(e.nativeEvent));
  };
  const onUp = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!draft) return;
    const end = pos(e.nativeEvent);
    const w = Math.abs(end.x - draft.x);
    const h = Math.abs(end.y - draft.y);
    setDraft(null);
    // Hairline drags are noise, not boxes.
    if (w < 0.03 || h < 0.03) return;
    onChange([
      ...boxes,
      {
        x: Math.min(draft.x, end.x),
        y: Math.min(draft.y, end.y),
        w,
        h,
        category: CATEGORIES[categoryIndex],
      },
    ]);
  };

  const removeSelected = () => {
    if (selected == null) return;
    onChange(boxes.filter((_, i) => i !== selected));
    setSelected(null);
  };

  return (
    <div data-testid="box-editor" className="space-y-1">
      <svg
        data-testid="box-canvas"
        width={EDITOR_W}
        height={EDITOR_H}
        className="touch-none rounded border border-zinc-700 bg-zinc-950"
        onPointerDown={onDown}
        onPointerUp={onUp}
        role="img"
        aria-label="frame with editable bounding boxes"
      >
        {boxes.map((box, i) => (
          <rect
            key={`${box.x}-${box.y}-${i}`}
            data-testid={`box-${i}`}
            x={box.x * EDITOR_W}
            y={box.y * EDITOR_H}
            width={box.w * EDITOR_W}
            height={box.h * EDITOR_H}
            fill="none"
            stroke={stroke(i, box.category)}
            strokeWidth={selected === i ? 4 : 3}
            onClick={(e) => {
              e.stopPropagation();
              setSelected(i);
            }}
          />
        ))}
      </svg>
      <p className="font-mono text-[10px] text-zinc-400">
        {boxes.length} box(es) — drag to add a {CATEGORIES[categoryIndex]}, click one
        to select, then remove.
      </p>
      <button
        type="button"
        data-testid="box-remove"
        onClick={removeSelected}
        disabled={selected == null}
        className="rounded border border-zinc-700 px-2 py-0.5 text-[11px] text-zinc-300 disabled:opacity-40"
      >
        remove selected
      </button>
    </div>
  );
}
