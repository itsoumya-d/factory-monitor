import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import BottleneckHeatmap from "@/components/BottleneckHeatmap";
import ReviewPage from "@/app/review/page";
import type { LabelRow } from "@/components/LabelApprover";
import { fromHourlyPoints, goalPerHour, hourlySeries, lowConfidenceWindows, zoneList, zoneKey } from "@/lib/series";
import type { HourlyPoint } from "@/lib/series";

vi.mock("@/lib/live", () => ({
  apiFetch: vi.fn(),
  useLive: ((sel: (s: { labels: unknown[] }) => number) => sel({ labels: [] })) as never,
}));
import { apiFetch } from "@/lib/live";

afterEach(() => {
  cleanup();
  vi.mocked(apiFetch).mockReset();
});

const wrap = (ui: ReactNode) => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
};

/** Real hour shape from GET /api/v1/zones/{z}/hourly (docs/10 §3). */
const SERVER_HOURS: HourlyPoint[] = [
  { hour: "2026-10-18T14:00:00Z", packages: 14, avg_conf: 0.93, occluded_events: 0 },
  { hour: "2026-10-18T15:00:00Z", packages: 3, avg_conf: 0.4, occluded_events: 1 },
];

describe("series contract with the Rust hourly rollup", () => {
  it("flattens server hours into zone rows", () => {
    const rows = fromHourlyPoints("line_1", SERVER_HOURS);
    expect(rows).toEqual([
      { zone_id: "line_1", hour: "14", delta: 14, avg_conf: 0.93 },
      { zone_id: "line_1", hour: "15", delta: 3, avg_conf: 0.4 },
    ]);
    expect(lowConfidenceWindows(rows)).toEqual([
      { zone_id: "line_1", hour: "15", avg_conf: 0.4 },
    ]);
    expect(zoneList(rows)).toEqual(["line_1"]);
    const series = hourlySeries(rows);
    expect(series[14][zoneKey("line_1")]).toBe(14);
    expect(series[0][zoneKey("line_1")]).toBe(0);
    expect(series).toHaveLength(24);
    expect(goalPerHour(48)).toBe(2);
  });
});

describe("BottleneckHeatmap", () => {
  it("ramps cells calm→storm and names the hottest window", () => {
    render(
      <BottleneckHeatmap
        rows={[
          { zone_id: "line_1", hour: "08", delta: 40 },
          { zone_id: "line_1", hour: "09", delta: 3 },
          { zone_id: "dock_gate", hour: "10", delta: 0 },
        ]}
      />,
    );
    expect(screen.getByTestId("heat-line_1-08").dataset.ramp).toBe("storm");
    expect(screen.getByTestId("heat-line_1-09").dataset.ramp).toBe("calm");
    expect(screen.getByTestId("heat-dock_gate-10").dataset.ramp).toBe("calm");
    expect(screen.getByTestId("heatmap-body").children).toHaveLength(2);
    expect(screen.getByTestId("heatmap-busiest").textContent).toContain(
      "line_1 at 08:00 (40 pkgs)",
    );
  });
});

const ROW: LabelRow = {
  frame_id: "0190-1234",
  zone_id: "line_1",
  ts: "2026-10-18T09:00:00Z",
  camera_id: "camA",
  divergence: 0.42,
  uncertainty_mask: 1,
  promotion: "Amber",
  coco_annotations: null,
};

describe("Review page", () => {
  it("lists the queue and approves with the operator id", async () => {
    // Server-side model: frames leave the queue once approved.
    const queue = [ROW, { ...ROW, frame_id: "0190-5678" }];
    const mock = vi.mocked(apiFetch);
    mock.mockImplementation((path, init) => {
      const url = typeof path === "string" ? path : String(path);
      if (url.endsWith("/labeling/queue")) {
        return Promise.resolve({ queue: [...queue] }) as never;
      }
      const match = /\/labeling\/([^/]+)\/approve$/.exec(url);
      if (match) {
        const body = JSON.parse((init as RequestInit).body as string) as {
          operator_id: string;
        };
        expect(body).toEqual({ operator_id: "op_1", coco_annotations: null });
        const i = queue.findIndex((r) => r.frame_id === match[1]);
        expect(i).toBeGreaterThanOrEqual(0);
        queue.splice(i, 1);
        return Promise.resolve({ frame_id: "0190-1234", approved: true }) as never;
      }
      return Promise.reject(new Error(`unexpected path: ${url}`)) as never;
    });

    wrap(<ReviewPage />);
    await waitFor(() => expect(screen.getAllByTestId(/^label-/)).toHaveLength(2));

    // Approve is gated on the operator id.
    expect(screen.getByTestId("approve-0190-1234").hasAttribute("disabled")).toBe(true);
    fireEvent.change(screen.getByTestId("operator-input"), { target: { value: "op_1" } });
    fireEvent.click(screen.getByTestId("approve-0190-1234"));

    await waitFor(() => {
      expect(screen.queryByTestId("label-0190-1234")).toBeNull();
    });
    // Other rows survive.
    expect(screen.getByTestId("label-0190-5678")).toBeTruthy();
    const call = mock.mock.calls.find((c) => String(c[0]).includes("/approve"))!;
    expect(String(call[0])).toBe("/api/v1/labeling/0190-1234/approve");
    expect(call[1]?.method).toBe("POST");
  });
});

describe("Review page reject verb", () => {
  it("posts {operator_id, reason} and the frame leaves the queue", async () => {
    const queue = [ROW];
    const mock = vi.mocked(apiFetch);
    mock.mockImplementation((path, init) => {
      const url = typeof path === "string" ? path : String(path);
      if (url.endsWith("/labeling/queue")) {
        return Promise.resolve({ queue: [...queue] }) as never;
      }
      const match = /\/labeling\/([^/]+)\/reject$/.exec(url);
      if (match) {
        const body = JSON.parse((init as RequestInit).body as string) as {
          operator_id: string;
          reason: string;
        };
        expect(body.operator_id).toBe("op_1");
        expect(body.reason.length).toBeGreaterThan(0);
        queue.splice(
          queue.findIndex((r) => r.frame_id === match[1]),
          1,
        );
        return Promise.resolve({ frame_id: match[1], rejected: true }) as never;
      }
      return Promise.reject(new Error(`unexpected path: ${url}`)) as never;
    });

    wrap(<ReviewPage />);
    await waitFor(() => expect(screen.getAllByTestId(/^label-/)).toHaveLength(1));

    fireEvent.change(screen.getByTestId("operator-input"), { target: { value: "op_1" } });
    fireEvent.click(screen.getByTestId(`reject-0190-1234`));
    await waitFor(() => {
      expect(screen.queryByTestId("label-0190-1234")).toBeNull();
    });
    await waitFor(() => {
      expect(screen.getByTestId("review-list").textContent).toContain("queue clear");
    });
    const call = mock.mock.calls.find((c) => String(c[0]).includes("/reject"))!;
    expect(String(call[0])).toBe("/api/v1/labeling/0190-1234/reject");
    expect(call[1]?.method).toBe("POST");
  });
});
