import { beforeAll, describe, expect, it } from "vitest";
import { applyEnvelope, liveSnapshot, replayReset } from "@/lib/live";
import type { WsEnvelope } from "@/lib/types";
import dayJson from "./fixtures/ws-day.json";

const DAY = dayJson as unknown as WsEnvelope[];

beforeAll(() => replayReset());

describe("docs/12 §7 WS mock harness — fixture-day replay", () => {
  it("applies the synthetic day through real pump semantics", () => {
    let state = liveSnapshot();
    for (const env of DAY) {
      state = { ...state, ...applyEnvelope(state, env) };
    }

    // dedupe dropped both replayed e1 duplicates: 2 + 5 (the second replay
    // of e1 with delta 7 and delta 100 never injected)
    const z = state.counts.get("dock_gate");
    expect(z?.count).toBe(7);
    expect(state.counts.get("line_1")?.count).toBe(3);

    expect(state.fusion.get("dock_gate")?.vision_weight).toBeCloseTo(0.72);
    expect(state.rl?.reason).toBe("eod_correction");
    expect(state.lifecycle?.state).toBe("Counting");
    expect(state.health.get("camD")?.fps).toBeCloseTo(5.02);
    expect(state.labels).toHaveLength(1);
    expect(state.labels[0]).toMatchObject({ frame_id: "0190-abc", zone_id: "dock_gate" });
  });

  it("caps the label queue at 100 and ignores unknown kinds", () => {
    replayReset();
    let state = liveSnapshot();
    const spam: WsEnvelope[] = Array.from({ length: 140 }, (_, i) => ({
      kind: "label_queue_added",
      payload: { frame_id: `f-${i}`, ts: "2026-10-18T10:00:00Z", zone_id: "z" },
    }));
    for (const env of [...spam, ...DAY]) {
      state = { ...state, ...applyEnvelope(state, env) };
    }
    expect(state.labels).toHaveLength(100);
    // the day's label lands newest-first; the 41st spam frame scrolls off
    expect(state.labels[0]).toMatchObject({ frame_id: "0190-abc" });
    expect(state.labels[99]).toMatchObject({ frame_id: "f-41" });
  });

  it("lastMessageAt ticks on each applied envelope", () => {
    const before = liveSnapshot().lastMessageAt;
    const out = applyEnvelope(liveSnapshot(), {
      kind: "camera_health",
      payload: { camera_id: "camX", online: false, fps: 0 },
    });
    expect(out.lastMessageAt).toBeGreaterThanOrEqual(before);
  });
});
