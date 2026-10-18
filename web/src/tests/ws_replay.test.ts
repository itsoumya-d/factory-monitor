import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useLive } from "@/lib/live";
import type { CountEvent } from "@/lib/types";

function countEvent(zone: string, delta: number, id: string): CountEvent {
  return {
    event_id: id,
    zone_id: zone,
    ts_from: "2026-10-18T08:00:00Z",
    ts_to: "2026-10-18T08:01:00Z",
    count_delta: delta,
    cumulative_after: delta,
    confidence: 0.93,
    uncertainty: "MODALITY_DIVERGENCE",
    modality_blend: { vision: 0.7, wifi: 0.3 },
    thresholds_used: { conf_threshold: 0.45, iou_threshold: 0.45, track_buffer_frames: 45 },
    track_ids: [7],
    model_version: "engine:test",
  };
}

/** Fake browser WebSocket that captures the handlers export set. */
class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  sent: string[] = [];
  closed = false;

  constructor(url: string) {
    void url;
    FakeWebSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.closed = true;
    this.onclose?.();
  }
  emit(kind: string, payload: unknown) {
    this.onmessage?.({ data: JSON.stringify({ kind, payload }) });
  }
}

describe("live connection consumes the wire contract (real connect path)", () => {
  let wsOriginal: typeof WebSocket;
  beforeEach(() => {
    FakeWebSocket.instances = [];
    wsOriginal = globalThis.WebSocket;
    globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
    useLive.api.store.setState((s) => ({ ...s, counts: new Map(), connected: false }));
  });
  afterEach(() => {
    useLive.api.close();
    globalThis.WebSocket = wsOriginal;
  });

  it("accent deltas accumulate and dup ids drop", () => {
    useLive.api.connect("/ws/live");
    const sock = FakeWebSocket.instances[0];
    sock.onopen?.();
    sock.emit("count_event", countEvent("line_1", 10, "e1"));
    sock.emit("count_event", countEvent("line_1", 4, "e2"));
    sock.emit("count_event", countEvent("line_1", 9, "e2")); // duplicate
    const store = useLive.api.store.getState();
    expect(store.connected).toBe(true);
    expect(store.counts.get("line_1")?.count).toBe(14);
  });

  it("health, lifecycle and fusion land in their buckets", () => {
    useLive.api.connect("/ws/live");
    const sock = FakeWebSocket.instances[0];
    sock.emit("camera_health", { camera_id: "cam1", ok: false, fps: 0, dropped_ratio: 1 });
    sock.emit("lifecycle_event", { from: "Counting", to: "Degraded", ts: "t" });
    sock.emit("fusion_signal", {
      zone_id: "line_1",
      ts: "t",
      vision_rate: 0.4,
      wifi_rate: 0.38,
      divergence: 0.02,
    });
    const store = useLive.api.store.getState();
    expect(store.health.get("cam1")?.ok).toBe(false);
    expect(store.lifecycle?.to).toBe("Degraded");
    expect(store.fusion.get("line_1")?.divergence).toBeCloseTo(0.02);
  });

  it("close triggers a reconnect schedule (backoff, not spam)", () => {
    vi.useFakeTimers();
    useLive.api.connect("/ws/live");
    const sock = FakeWebSocket.instances[0];
    sock.onopen?.();
    sock.onclose?.(); // simulate server drop
    expect(useLive.api.store.getState().attempts).toBe(1);
    vi.advanceTimersByTime(999);
    expect(FakeWebSocket.instances.length).toBe(1);
    vi.advanceTimersByTime(2);
    expect(FakeWebSocket.instances.length).toBe(2);
    vi.useRealTimers();
  });
});
