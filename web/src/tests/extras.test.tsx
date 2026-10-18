import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import Display from "@/app/reports/[date]/ReportFor";
import PolicyPage from "@/app/settings/policy/page";
import ReviewPage from "@/app/review/page";
import type { LabelRow } from "@/components/LabelApprover";
import type { RlHistoryRow, SuspectWindow } from "@/lib/types";
import { reportCsv } from "@/lib/series";

vi.mock("@/lib/live", () => ({
  apiFetch: vi.fn(),
  useLive: ((sel: (s: Record<string, unknown>) => unknown) => sel({ labels: [], rl: null })) as never,
}));
import { apiFetch } from "@/lib/live";

afterEach(() => {
  cleanup();
  vi.mocked(apiFetch).mockReset();
});

const wrap = (ui: React.ReactNode) =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      {ui}
    </QueryClientProvider>,
  );

describe("SuspectWindowViewer (docs/12 §3)", () => {
  const WIN: SuspectWindow = {
    zone_id: "dock_gate",
    ts: "2026-10-18T10:00:00Z",
    focal_frame_id: "f0",
    frames: [
      { frame_id: "f0", ts: "2026-10-18T09:55:00Z", camera_id: "camD", uncertainty_mask: 1, divergence: null, promotion: "Amber", vlm_root_cause: "steam occlusion | 0.82" },
      { frame_id: "f1", ts: "2026-10-18T10:00:00Z", camera_id: "camD", uncertainty_mask: 0, divergence: 0.4, promotion: "Amber", vlm_root_cause: null },
    ],
  };

  const QUEUE_ROW: LabelRow = {
    frame_id: "0190-abc",
    zone_id: "dock_gate",
    ts: "2026-10-18T10:00:00Z",
    camera_id: "camD",
    divergence: 0.5,
    uncertainty_mask: 1,
    promotion: "Amber",
    coco_annotations: null,
  };

  it("renders tiles and shows uncertainty flags of the selected frame", async () => {
    vi.mocked(apiFetch).mockImplementation((path) => {
      const url = typeof path === "string" ? path : String(path);
      if (url.includes("/api/v1/snapshots/window")) {
        expect(url).toContain("zone_id=dock_gate");
        expect(url).toContain("span=1800");
        return Promise.resolve(WIN) as never;
      }
      if (url.includes("/labeling/queue")) {
        return Promise.resolve({ queue: [QUEUE_ROW] }) as never;
      }
      return Promise.reject(new Error(`unexpected path ${url}`)) as never;
    });

    wrap(<ReviewPage />);
    await waitFor(() => expect(screen.getAllByTestId(/^label-/)).toHaveLength(1));
    fireEvent.click(screen.getByTestId("window-0190-abc"));
    await waitFor(() => expect(screen.getByTestId("suspect-strip")).toBeTruthy());
    expect(screen.getAllByTestId(/^suspect-tile-/)).toHaveLength(2);
    // focal frame active by default → its flag chip (mask 1 = OCCLUDED_ESTIMATE)
    await waitFor(() => {
      expect(screen.getByTestId("suspect-flags").textContent).toContain("OCCLUDED_ESTIMATE");
    });
    // the flagged frame's root-cause text rides along; the clean frame hides it
    expect(screen.getByTestId("suspect-root-cause").textContent).toContain("steam occlusion");
    fireEvent.click(screen.getByTestId("suspect-tile-1"));
    await waitFor(() => {
      expect(screen.getByTestId("suspect-flags").textContent).toContain("no uncertainty flags");
    });
    expect(screen.queryByTestId("suspect-root-cause")).toBeNull();
    expect(screen.getByTestId("suspect-ts").textContent).toBe("2026-10-18T10:00:00Z");
    fireEvent.click(screen.getByTestId("suspect-close"));
    await waitFor(() => expect(screen.queryByTestId("suspect-window")).toBeNull());
  });

  it("shows the empty-window edge state", async () => {
    vi.mocked(apiFetch).mockImplementation((path) => {
      const url = typeof path === "string" ? path : String(path);
      if (url.includes("/api/v1/snapshots/window")) {
        return Promise.resolve({ ...WIN, frames: [], focal_frame_id: null }) as never;
      }
      if (url.includes("/labeling/queue")) {
        return Promise.resolve({ queue: [QUEUE_ROW] }) as never;
      }
      return Promise.reject(new Error(`unexpected path ${url}`)) as never;
    });
    wrap(<ReviewPage />);
    await waitFor(() => expect(screen.getAllByTestId(/^label-/)).toHaveLength(1));
    fireEvent.click(screen.getByTestId("window-0190-abc"));
    await waitFor(() => {
      expect(screen.getByTestId("suspect-empty").textContent).toContain("window is empty");
    });
    fireEvent.click(screen.getByTestId("suspect-close"));
    expect(screen.queryByTestId("suspect-window")).toBeNull();
  });
});

describe("RlPanel pause / revert / history (docs/12 §3)", () => {
  it("POSTs the override for preset durations and lists rl_audit rows", async () => {
    const calls: { url: string; body?: unknown }[] = [];
    vi.mocked(apiFetch).mockImplementation((path, init) => {
      const url = typeof path === "string" ? path : String(path);
      calls.push({ url, body: init?.body ? JSON.parse(init.body as string) : undefined });
      if (url === "/api/v1/rl/state") {
        return Promise.resolve({ paused_until: null, override_note: null, rejected_24h: 0 }) as never;
      }
      if (url.startsWith("/api/v1/rl/history?date=")) {
        const row: RlHistoryRow = {
          transition_id: "t1",
          ts: "2026-10-18T09:00:00Z",
          zone_id: null,
          before: { conf_threshold: 0.45, iou_threshold: 0.45, vision_weight: 0.7, track_buffer_frames: 45 },
          after: { conf_threshold: 0.51, iou_threshold: 0.5, vision_weight: 0.72, track_buffer_frames: 60 },
          reward: -0.65,
          was_clamped: false,
          rejected: false,
          reason: "eod_correction",
        };
        return Promise.resolve({ date: row.ts.slice(0, 10), transitions: [row] }) as never;
      }
      if (url === "/api/v1/rl/override") {
        return Promise.resolve({ ok: true }) as never;
      }
      return Promise.reject(new Error(`unexpected path ${url}`)) as never;
    });

    wrap(<PolicyPage />);

    await waitFor(() => expect(screen.getByTestId("rl-history")).toBeTruthy());
    expect(screen.getByTestId("rl-history").textContent).toContain("0.45");
    expect(screen.getByTestId("rl-history").textContent).toContain("eod_correction");

    fireEvent.click(screen.getByTestId("pause-2-hours"));
    await waitFor(() => {
      expect(calls.find((c) => c.url === "/api/v1/rl/override")?.body).toMatchObject({
        note: "manual pause (2 hours)",
      });
    });
    const pausedBody = calls.find((c) => c.url === "/api/v1/rl/override")?.body as { until: string };
    expect(new Date(pausedBody.until).getTime()).toBeGreaterThan(Date.now());

    fireEvent.click(screen.getByTestId("revert-button"));
    await waitFor(() => {
      const revertBody = calls[calls.length - 1];
      expect(revertBody.url).toBe("/api/v1/rl/override");
      expect(String((revertBody.body as { note: string }).note)).toContain("revert to last-safe");
      expect(new Date((revertBody.body as { until: string }).until).getTime()).toBeGreaterThan(Date.now());
    });
  });
});

describe("Report exports + windows-to-review (docs/12 §5)", () => {
  it("derives LOW_CONFIDENCE windows, exports CSV, copies webhook payload", async () => {
    const csvWrites: string[] = [];
    const clicked: string[] = [];
    // stub blob download + clipboard (jsdom has no real clipboard)
    const realCreate = document.createElement.bind(document);
    vi.spyOn(URL, "createObjectURL").mockImplementation((() => {
      csvWrites.push("blob");
      return "blob:fake";
    }) as never);
    URL.revokeObjectURL = () => undefined;
    vi.spyOn(document, "createElement").mockImplementation(((tag: string) => {
      const el = realCreate(tag);
      if (tag === "a") {
        el.click = () => clicked.push("a-click");
      }
      return el as HTMLElement;
    }) as never);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
      configurable: true,
    });

    const pending = {
      pending: [
        {
          correction_id: "c1",
          feedback: { zone_id: "line_1", reported_count: 10, actual_count: 12, ts_from: "2026-10-18T10:00:00Z", ts_to: "2026-10-18T11:00:00Z", failure_tags: 0, notes: "", operator_id: "op", wants_retrain: false },
          outcome: "submitted",
          reward_computed: -0.02,
          root_cause_report: null,
          created_at: "2026-10-18T09:30:00Z",
        },
      ],
    };
    vi.mocked(apiFetch).mockImplementation((path) => {
      const url = typeof path === "string" ? path : String(path);
      if (url === "/api/v1/audit/pending") return Promise.resolve(pending) as never;
      if (url.includes("/api/v1/zones/line_1/hourly")) {
        return Promise.resolve({
          zone_id: "line_1",
          date: "2026-10-18",
          hours: [
            { hour: "2026-10-18T10:00:00Z", packages: 14, avg_conf: 0.93, occluded_events: 0 },
            { hour: "2026-10-18T15:00:00Z", packages: 3, avg_conf: 0.4, occluded_events: 1 },
          ],
        }) as never;
      }
      if (url.includes("/snapshots/window")) {
        return Promise.resolve({ zone_id: "line_1", ts: "2026-10-18T15:00:00Z", focal_frame_id: null, frames: [] }) as never;
      }
      return Promise.reject(new Error(`unexpected path ${url}`)) as never;
    });

    wrap(<Display date="2026-10-18" />);
    await waitFor(() => expect(screen.getByTestId("windows-to-review")).toBeTruthy());

    // windows-to-review timeline: only the 0.4-avg_conf hour falls below 0.75
    await waitFor(() => expect(screen.getByTestId("window-line_1-15")).toBeTruthy());
    fireEvent.click(screen.getByTestId("window-line_1-15"));
    await waitFor(() => expect(screen.getByTestId("suspect-empty")).toBeTruthy());
    fireEvent.click(screen.getByTestId("suspect-close"));

    fireEvent.click(screen.getByTestId("export-csv"));
    await waitFor(() => {
      expect(screen.getByTestId("export-note").textContent).toContain("csv downloaded");
    });
    expect(csvWrites).toContain("blob");
    fireEvent.click(screen.getByTestId("copy-webhook"));
    await waitFor(() => {
      expect(screen.getByTestId("copy-webhook").textContent).toContain("copied");
    });
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      expect.stringContaining('"date":"2026-10-18"'),
    );
    // CSV body matches the wire shape
    const csv = reportCsv(
      [
        { hour: "10", zone_id: "line_1", delta: 14, avg_conf: 0.93 },
        { hour: "15", zone_id: "line_1", delta: 3, avg_conf: 0.4 },
      ],
      "2026-10-18",
    );
    expect(csv.split("\n")[0]).toBe("zone_id,hour,packages,avg_conf,date");
    expect(csv.split("\n")[2]).toBe("line_1,15,3,0.400,2026-10-18");
  });
});
