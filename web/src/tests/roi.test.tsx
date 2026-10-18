import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import RoiEditor, { CANVAS_PX, CLICK_RADIUS } from "@/components/RoiEditor";
import { roiSubmitBody, segmentsIntersect, validateRoiLines } from "@/lib/roi";

afterEach(cleanup);

const ZONES = ["line_1", "dock_gate"];
const p = (x: number, y: number) => ({ x, y });
const LINE = { points: [p(0.1, 0.1), p(0.9, 0.4)] };

/** Canvas coords are (norm * CANVAS_PX); jsdom rects are zero-origin. */
const clickAt = (canvas: Element, u: number, v: number) =>
  fireEvent.click(canvas, { clientX: u * CANVAS_PX, clientY: v * CANVAS_PX });

describe("segments_intersect mirrors inference_engine::zones (parity pins)", () => {
  it("proper crossings true", () => {
    expect(segmentsIntersect(p(0, 0), p(10, 10), p(0, 10), p(10, 0))).toBe(true);
  });
  it("endpoint touch counts (Rust parity)", () => {
    expect(segmentsIntersect(p(0, 0), p(10, 0), p(10, -5), p(10, 5))).toBe(true);
  });
  it("nearest misses false", () => {
    expect(segmentsIntersect(p(0, 0), p(10, 10), p(11, 0), p(11, 5))).toBe(false);
  });
  it("collinear overlap true (touching/collinear branch)", () => {
    expect(segmentsIntersect(p(0, 0), p(5, 0), p(3, 0), p(8, 0))).toBe(true);
  });
});

describe("roi REST body contract", () => {
  it("matches the Rust RoiSubmit shape", () => {
    expect(roiSubmitBody([LINE, LINE])).toEqual({
      lines: [LINE.points, LINE.points],
    });
  });
  it("rejects what the Rust side would reject", () => {
    expect(validateRoiLines([])).toBe("lines must be non-empty");
    expect(validateRoiLines([{ points: [p(0.1, 0.1)] }])).toBe(
      "a counting line needs at least two points",
    );
    expect(validateRoiLines([{ points: [p(0.5, 0.5), p(1.2, 0.3)] }])).toBe(
      "points must be finite in [0,1]",
    );
    expect(() => roiSubmitBody([{ points: [] }])).toThrow();
  });
});

describe("RoiEditor interaction", () => {
  it("ropes a line from clicks and only submits valid zones", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockImplementation(() =>
      Promise.resolve(
        new Response(JSON.stringify({ zone_id: "line_1", applied: true, lines: 1 }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      ) as unknown as Promise<Response>,
    );

    render(<RoiEditor zones={ZONES} />);
    const canvas = screen.getByTestId("roi-canvas");

    // One pending point: submit must stay disabled.
    clickAt(canvas, 0.25, 0.25);
    expect(screen.getByTestId("roi-submit").hasAttribute("disabled")).toBe(true);
    expect(screen.getByTestId("roi-status").textContent).toContain("1 pending");

    // Second point finishes the line.
    clickAt(canvas, 0.75, 0.25);
    expect(screen.getByTestId("roi-submit").hasAttribute("disabled")).toBe(false);
    expect(screen.getByTestId("roi-line-0")).toBeTruthy();

    fireEvent.click(screen.getByTestId("roi-submit"));
    await waitFor(() => {
      expect(screen.getByTestId("roi-applied").textContent).toContain("line_1: applied 1");
    });
    const [url, init] = fetchSpy.mock.calls[0];
    expect(String(url)).toBe("/api/v1/calibration/rois/line_1");
    expect(JSON.parse(init?.body as string)).toEqual({
      lines: [[p(0.25, 0.25), p(0.75, 0.25)]],
    });

    fetchSpy.mockRestore();
  });

  it("grip-click pops the last point (undo)", () => {
    render(<RoiEditor zones={ZONES} />);
    const canvas = screen.getByTestId("roi-canvas");
    clickAt(canvas, 0.25, 0.25);
    clickAt(canvas, 0.75, 0.25);
    // Click within the handle radius of the last point → pop.
    clickAt(canvas, 0.75 + CLICK_RADIUS / 2, 0.25);
    expect(screen.getByTestId("roi-line-0")).toBeTruthy();
    expect(screen.getByTestId("roi-handle-0-0")).toBeTruthy();
    expect(screen.queryByTestId("roi-handle-0-1")).toBeNull();
    expect(screen.getByTestId("roi-status").textContent).toContain("1 pending");
  });

  it("end line opens a fresh rope", () => {
    render(<RoiEditor zones={ZONES} />);
    const canvas = screen.getByTestId("roi-canvas");
    clickAt(canvas, 0.1, 0.1);
    clickAt(canvas, 0.6, 0.6);
    fireEvent.click(screen.getByTestId("roi-end-line"));
    expect(screen.getByTestId("roi-line-0")).toBeTruthy();
    expect(screen.getByTestId("roi-line-1")).toBeTruthy();
    clickAt(canvas, 0.2, 0.8);
    // rope 1 pending until second point; both strokes visible
    expect(screen.getByTestId("roi-handle-1-0")).toBeTruthy();
  });
});
