import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import BoxEditor, { CATEGORIES, EDITOR_H, EDITOR_W } from "@/components/BoxEditor";
import { cocoApproveBody, imageUrl, packageCoco, type CocoDataset, type DraftBox } from "@/lib/coco";
import type { LabelRow } from "@/components/LabelApprover";
import LabelApprover from "@/components/LabelApprover";

afterEach(cleanup);

describe("coco.ts mirrors vlm_orchestrator::package_coco", () => {
  it("packs 1-indexed ids, pixel bboxes, area and fail-closed taxonomy", () => {
    const coco = packageCoco({
      frameId: "0190",
      fileName: "0190.jpg",
      width: 640,
      height: 480,
      boxes: [
        { x: 0.1, y: 0.2, w: 0.25, h: 0.5, category: "package" },
        { x: 0.5, y: 0.5, w: 0.125, h: 0.25, category: "not_in_taxonomy" },
        { x: 0.75, y: 0.75, w: 0.1, h: 0.1, category: "pallet" },
      ],
      categories: [...CATEGORIES],
    });
    // From the Rust pins: images[{id:1,…}], annotations[{id:1,…},…], cat 1-index.
    expect(coco.images).toEqual([{ id: 1, file_name: "0190.jpg", height: 480, width: 640 }]);
    expect(coco.annotations).toEqual([
      { id: 1, image_id: 1, category_id: 1, bbox: [64, 96, 160, 240], area: 160 * 240, iscrowd: 0 },
      { id: 2, image_id: 1, category_id: 2, bbox: [480, 360, 64, 48], area: 64 * 48, iscrowd: 0 },
    ]);
    expect(coco.categories).toEqual(["package", "pallet"]);
  });

  it("zeros stay stable and the approve body keys match the extractor", () => {
    const empty = packageCoco({
      frameId: "f",
      fileName: "f.jpg",
      width: 640,
      height: 480,
      boxes: [],
      categories: [...CATEGORIES],
    });
    expect(empty.annotations).toEqual([]);
    expect(cocoApproveBody("op_1", empty)).toEqual({
      operator_id: "op_1",
      coco_annotations: empty,
    });
  });

  it("the image endpoint is the only byte route and percent-safes ids", () => {
    expect(imageUrl("0190-abcd")).toBe("/api/v1/snapshots/image/0190-abcd");
    expect(imageUrl("a/b c")).toBe("/api/v1/snapshots/image/a%2Fb%20c");
  });
});

describe("BoxEditor", () => {
  /** jsdom derives offsetX from clientX minus the (zero) layout offset —
   *  same rule the ROI tests pass on, so strokes go through clientX. */
  const px = (u: number, v: number) => ({ clientX: u * EDITOR_W, clientY: v * EDITOR_H });

  it("drags a box and removes it, keeping pixel-space geometry", () => {
    const original: DraftBox[] = [
      { x: 0.1, y: 0.1, w: 0.2, h: 0.2, category: "package" },
    ];
    const seen: DraftBox[][] = [];
    render(
      <BoxEditor categoryIndex={0} boxes={original} onChange={(b) => seen.push(b)} />,
    );
    const canvas = screen.getByTestId("box-canvas");


    fireEvent.pointerDown(canvas, px(0.25, 60 / EDITOR_H));
    fireEvent.pointerMove(canvas, px(0.6, 150 / EDITOR_H));
    fireEvent.pointerUp(canvas, px(0.625, 150 / EDITOR_H));

    expect(seen).toHaveLength(1);
    expect(seen[0]).toHaveLength(2);
    expect(seen[0][1]).toEqual({ x: 0.25, y: 0.25, w: 0.375, h: 0.375, category: "package" });

    // Hairline drags are noise and commit nothing.
    fireEvent.pointerDown(canvas, px(0.5, 0.5));
    fireEvent.pointerUp(canvas, px(0.501, 0.501));
    expect(seen.every((b) => b.length === 2)).toBe(true);
  });

  it("selects and removes a box", () => {
    const boxes = [{ x: 0.2, y: 0.2, w: 0.4, h: 0.3, category: "pallet" }];
    const seen: DraftBox[][] = [];
    render(<BoxEditor categoryIndex={1} boxes={boxes} onChange={(b) => seen.push(b)} />);
    const box = screen.getByTestId("box-0");
    fireEvent.click(box);
    fireEvent.click(screen.getByTestId("box-remove"));
    expect(seen).toEqual([[]]);
  });
});

describe("LabelApprover draws-on-approve flow", () => {
  const ROW: LabelRow = {
    frame_id: "0190-x",
    zone_id: "dock_gate",
    ts: "2026-10-18T09:00:00Z",
    camera_id: "camD",
    divergence: 0.42,
    uncertainty_mask: 1,
    promotion: "Amber",
    coco_annotations: null,
  };

  it("preloads VLM boxes from the stored dataset and posts the fixed COCO", async () => {
    const stored: CocoDataset = {
      images: [{ id: 1, file_name: "0190-x.jpg", height: 480, width: 640 }],
      annotations: [
        { id: 1, image_id: 1, category_id: 2, bbox: [32, 48, 320, 240], area: 320 * 240, iscrowd: 0 },
      ],
      categories: ["package", "pallet"],
    };
    const posts: { url: string; body: Record<string, unknown> }[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(
      (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes("/approve")) {
          posts.push({ url, body: JSON.parse(init?.body as string) as Record<string, unknown> });
          return new Response(JSON.stringify({ approved: true }), {
            status: 200,
            headers: { "content-type": "application/json" },
          }) as unknown as Response;
        }
        return new Response("not found", { status: 404 }) as unknown as Response;
      }) as unknown as (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>,
    );

    render(<LabelApprover row={{ ...ROW, coco_annotations: stored }} operator="op_1" onDone={() => {}} />);
    fireEvent.click(screen.getByTestId("inspect-0190-x"));
    // Preloaded box normalized: 32/640 = 0.05.
    expect(screen.getByTestId("box-0").getAttribute("x")).toBe("16"); // 0.05*EDITOR_W
    fireEvent.click(screen.getByTestId("approve-0190-x"));

    const body = posts[0]?.body as { operator_id: string; coco_annotations: CocoDataset };
    expect(body.operator_id).toBe("op_1");
    expect(body.coco_annotations?.annotations[0]).toEqual({
      id: 1,
      image_id: 1,
      category_id: 2,
      bbox: [32, 48, 320, 240],
      area: 320 * 240,
      iscrowd: 0,
    });
    vi.restoreAllMocks();
  });
});
