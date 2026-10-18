import type { CountEvent, UncertaintyFlag } from "./types";

/** Typed name for every uncertainty flag known to the backend (bitflags Display). */
export const UNCERTAINTY_NAMES: UncertaintyFlag[] = [
  "OCCLUDED_ESTIMATE",
  "MODALITY_DIVERGENCE",
  "CLOCK_DEGRADED",
  "LOW_TRACK_STRENGTH",
];

/**
 * Parse the backend's uncertainty representation. The Rust side serializes
 * bitflags as a Display string ("OCCLUDED_ESTIMATE | MODALITY_DIVERGENCE");
 * tolerate also omissions and masked integers.
 */
export function parseUncertainty(raw: string): UncertaintyFlag[] {
  let names: string[] = [];
  if (typeof raw === "number") {
    names = UNCERTAINTY_NAMES.filter((_, bit) => (1 << bit) & raw);
  } else {
    const trimmed = raw.trim();
    if (trimmed && trimmed !== "0x0" && trimmed !== "0") names = trimmed.split("|").map((s) => s.trim());
  }
  return names.filter((n): n is UncertaintyFlag => (UNCERTAINTY_NAMES as string[]).includes(n));
}

export type ConfLevel = "green" | "amber" | "red";

/** docs/12 §3 ConfBadge: green ≥0.9 / amber ≥0.75 / red else. */
export function confidenceLevel(c: number): ConfLevel {
  if (c >= 0.9) return "green";
  if (c >= 0.75) return "amber";
  return "red";
}

/**
 * docs/12 §3 CorrectionForm "Bad-delta guard": a delta larger than 25% of
 * the predicted count asks "Confirm big swing?" and wants the retrain box.
 */
export function needsBigSwingConfirmation(reported: number, actual: number): boolean {
  if (reported === 0) return actual !== 0;
  return Math.abs(actual - reported) / Math.abs(reported) > 0.25;
}

/** Failure tag names mirroring common_types::feedback::FailureTag (bit order). */
export const FAILURE_TAGS = [
  "MISSED_COUNT",
  "DOUBLE_COUNT",
  "PALLET_AS_BOX",
  "OCCLUSION_LIGHTING",
  "OTHER",
] as const;

export type FailureTagName = (typeof FAILURE_TAGS)[number];

/** Encode checked tag names into the backend's u16 mask. */
export function encodeFailureTags(checked: string[]): number {
  let mask = 0;
  FAILURE_TAGS.forEach((name, i) => {
    if (checked.includes(name)) mask |= 1 << i;
  });
  return mask;
}

/** Safe integer pool: fewer than 14-bit masks. */
export function encodeSafe(checked: string[]): number {
  const mask = encodeFailureTags(checked);
  return mask < 1 << FAILURE_TAGS.length ? mask : 0;
}

/** The reward preview the backend computes: -|actual - reported| / 100. */
export function rewardPreview(reported: number, actual: number): number {
  return -Math.abs(actual - reported) / 100;
}

/**
 * Reconnection schedule per docs/12 §4: exponential 1s→30s, capped.
 */
export function backoffMs(attempt: number): number {
  const ms = Math.min(30000, 1000 * 2 ** Math.max(0, attempt));
  return ms;
}

/** Stale-data banner threshold (docs/12 §4: last_message_age > 10s). */
export const STALE_AFTER_MS = 10_000;

/**
 * Dedupe guard for live events (docs/12 §4: drop duplicate event_ids).
 */
export class EventDedupe {
  private seen = new Set<string>(new Array(0));

  accept(id: string): boolean {
    if (this.seen.has(id)) return false;
    this.seen.add(id);
    if (this.seen.size > 4096) {
      // Drop the oldest halves-ish; bounded memory for a fat day.
      const keys = [...this.seen];
      this.seen = new Set(keys.slice(keys.length / 2));
    }
    return true;
  }
}

/** Sum delta counts into a per-zone reduce (CounterTicker). */
export function zoneTotals(events: CountEvent[]): Map<string, number> {
  const m = new Map<string, number>();
  for (const e of events) m.set(e.zone_id, (m.get(e.zone_id) ?? 0) + e.count_delta);
  return m;
}
