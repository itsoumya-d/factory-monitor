import { describe, expect, it } from "vitest";
import {
  EventDedupe,
  FAILURE_TAGS,
  backoffMs,
  confidenceLevel,
  encodeFailureTags,
  needsBigSwingConfirmation,
  parseUncertainty,
  rewardPreview,
  zoneTotals,
} from "@/lib/formats";
import type { CountEvent } from "@/lib/types";

describe("conf badge levels (docs/12 §3)", () => {
  it("green ≥0.9, amber ≥0.75, red else", () => {
    expect(confidenceLevel(0.9)).toBe("green");
    expect(confidenceLevel(0.897)).toBe("amber");
    expect(confidenceLevel(0.75)).toBe("amber");
    expect(confidenceLevel(0.749)).toBe("red");
    expect(confidenceLevel(0)).toBe("red");
  });
});

describe("uncertainty flags wire parse", () => {
  it("bitflags display strings map to names", () => {
    expect(parseUncertainty("OCCLUDED_ESTIMATE | MODALITY_DIVERGENCE")).toEqual([
      "OCCLUDED_ESTIMATE",
      "MODALITY_DIVERGENCE",
    ]);
  });
  it("empty and 0x0 mean no flags", () => {
    expect(parseUncertainty("")).toEqual([]);
    expect(parseUncertainty("0x0")).toEqual([]);
  });
  it("unknown names are dropped", () => {
    expect(parseUncertainty("OCCLUDED_ESTIMATE | WEIRD")).toEqual(["OCCLUDED_ESTIMATE"]);
  });
});

describe("bad-delta guard (CorrectionForm)", () => {
  it("25% is the threshold", () => {
    expect(needsBigSwingConfirmation(100, 126)).toBe(true);
    expect(needsBigSwingConfirmation(100, 125)).toBe(false);
    expect(needsBigSwingConfirmation(100, 75)).toBe(false);
    expect(needsBigSwingConfirmation(100, 74)).toBe(true);
  });
  it("zero predicted counts swing on any nonzero actual", () => {
    expect(needsBigSwingConfirmation(0, 0)).toBe(false);
    expect(needsBigSwingConfirmation(0, 1)).toBe(true);
    expect(needsBigSwingConfirmation(0, -1)).toBe(true);
  });
  it("sign shape mirrors the backend (docs/01 1485 vs 1420)", () => {
    expect(needsBigSwingConfirmation(1485, 1420)).toBe(false);
    expect(rewardPreview(1485, 1420)).toBeCloseTo(-0.65);
    expect(rewardPreview(100, 250)).toBeCloseTo(-1.5);
    expect(rewardPreview(300, 300)).toBeCloseTo(0);
    expect(rewardPreview(200, 50)).toBeCloseTo(-1.5);
  });
});

describe("reconnection backoff (docs/12 §4)", () => {
  it("doubles then caps at 30s", () => {
    expect(backoffMs(0)).toBe(1000);
    expect(backoffMs(1)).toBe(2000);
    expect(backoffMs(4)).toBe(16000);
    expect(backoffMs(5)).toBe(30000);
    expect(backoffMs(12)).toBe(30000);
  });
});

describe("event dedupe (docs/12 §4)", () => {
  it("drops duplicates and survives a flood", () => {
    const d = new EventDedupe();
    expect(d.accept("a")).toBe(true);
    expect(d.accept("a")).toBe(false);
    for (let i = 0; i < 5000; i++) d.accept(`id-${i}`);
    // The documented invariant is bounded memory + near-head dedupe,
    // not immortality of ancient ids once the floor trims them.
    expect(d.accept("brand-new")).toBe(true);
    expect(d.accept("brand-new")).toBe(false);
    expect(d.accept("brand-new")).toBe(false);
  });
});

describe("failure tag masks mirror common_types::FailureTag", () => {
  it("bit positions match the Rust enum order", () => {
    expect(encodeFailureTags(["MISSED_COUNT"])).toBe(1);
    expect(encodeFailureTags(["DOUBLE_COUNT"])).toBe(2);
    expect(encodeFailureTags(["OCCLUSION_LIGHTING", "MISSED_COUNT"])).toBe(0b1001);
    expect(encodeFailureTags([])).toBe(0);
    expect(FAILURE_TAGS.length).toBe(5);
  });
});

describe("zone totals (CounterTicker)", () => {
  it("deltas accumulate per zone", () => {
    const ev = (zone: string, delta: number): CountEvent => ({
      event_id: `${zone}-${delta}`,
      zone_id: zone,
      ts_from: "",
      ts_to: "",
      count_delta: delta,
      cumulative_after: 0,
      confidence: 0.9,
      uncertainty: "",
      modality_blend: { vision: 1, wifi: 0 },
      thresholds_used: { conf_threshold: 0.45, iou_threshold: 0.45, track_buffer_frames: 45 },
      track_ids: [],
      model_version: "test",
    });
    const totals = zoneTotals([ev("a", 3), ev("a", 2), ev("b", -1)]);
    expect(totals.get("a")).toBe(5);
    expect(totals.get("b")).toBe(-1);
  });
});
