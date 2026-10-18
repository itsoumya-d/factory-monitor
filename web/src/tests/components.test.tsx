import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import ConfBadge from "@/components/ConfBadge";
import ZoneCard from "@/components/ZoneCard";
import RlPanel from "@/components/RlPanel";
import LifecycleStrip from "@/components/LifecycleStrip";
import CorrectionForm from "@/components/CorrectionForm";
import type { PendingWindow } from "@/lib/types";
import { useLive } from "@/lib/live";

afterEach(cleanup);

const WIN: PendingWindow = {
  window_id: "w1",
  zone_id: "line_1",
  ts_from: "2026-10-18T08:00:00Z",
  ts_to: "2026-10-18T08:15:00Z",
  predicted_count: 100,
  confidence: 0.8,
  reason: "LOW_CONFIDENCE",
};

describe("ConfBadge", () => {
  it("renders green/amber/red chips", () => {
    render(<ConfBadge value={0.95} />);
    expect(screen.getByTestId("conf-green")).toBeTruthy();
    render(<ConfBadge value={0.8} />);
    expect(screen.getByTestId("conf-amber")).toBeTruthy();
    render(<ConfBadge value={0.1} />);
    expect(screen.getByTestId("conf-red")).toBeTruthy();
  });
});

describe("ZoneCard", () => {
  it("shows count, conf and flags", () => {
    const live = {
      zoneId: "line_1",
      count: 14,
      lastEvent: {
        event_id: "1",
        zone_id: "line_1",
        ts_from: "",
        ts_to: "",
        count_delta: 14,
        cumulative_after: 14,
        confidence: 0.55,
        uncertainty: "OCCLUDED_ESTIMATE",
        modality_blend: { vision: 0.8, wifi: 0.2 },
        thresholds_used: { conf_threshold: 0.45, iou_threshold: 0.45, track_buffer_frames: 45 },
        track_ids: [],
        model_version: "t",
      },
      flagNames: [],
      flash: false,
    };
    render(<ZoneCard live={live} />);
    expect(screen.getByTestId("zone-count").textContent).toBe("14");
    expect(screen.getByTestId("conf-red")).toBeTruthy();
    expect(screen.getByText("occluded estimate")).toBeTruthy();
  });
});

describe("LifecycleStrip", () => {
  it("marks Degraded with a checklist hint", () => {
    useLive.api.store.setState((s) => ({
      ...s,
      lifecycle: { from: "Counting", to: "Degraded", ts: "2026-10-18T00:00:00Z" },
    }));
    render(<LifecycleStrip />);
    const strip = screen.getByTestId("lifecycle-strip");
    expect(strip.textContent).toContain("Degraded");
    expect(strip.textContent).toContain("check camera + CSI health");
  });
});

describe("RlPanel", () => {
  it("draws the runway with current values", () => {
    render(
      <RlPanel
        transition={{
          before: { conf_threshold: 0.44, iou_threshold: 0.45, vision_weight: 0.7, track_buffer_frames: 45 },
          after: { conf_threshold: 0.51, iou_threshold: 0.45, vision_weight: 0.7, track_buffer_frames: 45 },
          reason: "operator confirmed 1,485 vs 1,420",
          rejected: false,
        }}
      />,
    );
    expect(screen.getByTestId("rl-current-conf_threshold").textContent).toBe("0.51");
    expect(screen.getByTestId("rl-reason").textContent).toContain("1,485");
  });
});

describe("CorrectionForm bad-delta guard", () => {
  it("blocks >25% swings until confirm + retrain are ticked", async () => {
    const RESULT = { correction_id: "c1", reward_computed: -0.65, retrain: true };
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(() =>
        Promise.resolve(
          new Response(JSON.stringify(RESULT), {
            status: 200,
            headers: { "content-type": "application/json" },
          }),
        ) as unknown as Promise<Response>);

    render(<CorrectionForm windows={[WIN]} />);

    fireEvent.change(screen.getByTestId("zone-select"), {
      target: { value: "line_1" },
    });
    fireEvent.change(screen.getByTestId("window-select"), {
      target: { value: "w1" },
    });
    const submit = screen.getByTestId("submit-correction");
    expect(submit.hasAttribute("disabled")).toBe(true);

    fireEvent.change(screen.getByTestId("actual-input"), { target: { value: "250" } });
    expect(screen.getByTestId("swing-warning").textContent).toContain("25%");
    expect(submit.hasAttribute("disabled")).toBe(true);
    expect(screen.getByTestId("reward-preview").textContent).toContain("-1.50");

    fireEvent.click(screen.getByTestId("confirm-swing"));
    fireEvent.click(screen.getByTestId("wants-retrain"));
    expect(submit.hasAttribute("disabled")).toBe(false);

    fireEvent.click(submit);
    await waitFor(() => expect(screen.getByTestId("submit-result").textContent).toContain("c1"));
    const [url, init] = fetchSpy.mock.calls[0];
    expect(String(url)).toBe("/api/v1/audit/corrections");
    const body = JSON.parse(init?.body as string) as Record<string, unknown>;
    expect(body.actual_count).toBe(250);
    expect(body.wants_retrain).toBe(true);

    fetchSpy.mockRestore();
  });

  it("passes small swings without the guard", () => {
    render(<CorrectionForm windows={[WIN]} />);
    expect(screen.queryByTestId("swing-warning")).toBeFalsy();

    fireEvent.change(screen.getByTestId("zone-select"), { target: { value: "line_1" } });
    fireEvent.change(screen.getByTestId("window-select"), { target: { value: "w1" } });
    fireEvent.change(screen.getByTestId("actual-input"), { target: { value: "110" } });
    expect(screen.queryByTestId("swing-warning")).toBeFalsy();
  });
});
