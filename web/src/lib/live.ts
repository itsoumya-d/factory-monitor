import { create } from "zustand";
import type {
  CameraHealth,
  CountEvent,
  FusionSignal,
  LifecycleEvent,
  RlTransition,
  WsEnvelope,
} from "./types";
import { EventDedupe, STALE_AFTER_MS, backoffMs } from "./formats";

export interface ZoneLive {
  zoneId: string;
  count: number;
  lastEvent: CountEvent | null;
  flagNames: string[];
  flash: boolean;
}

export interface LiveState {
  connected: boolean;
  attempts: number;
  lastMessageAt: number;
  now: number;
  counts: Map<string, ZoneLive>;
  fusion: Map<string, FusionSignal>;
  rl: RlTransition | null;
  lifecycle: LifecycleEvent | null;
  health: Map<string, CameraHealth>;
  labels: { frame_id: string; ts: string; zone_id: string }[];
  peek(zoneId: string): ZoneLive | null;
  stale(): boolean;
}

const dedupe = new EventDedupe();

function zoneOf(s: Map<string, ZoneLive>, zoneId: string): ZoneLive {
  const hit = s.get(zoneId);
  if (hit) return hit;
  const fresh: ZoneLive = { zoneId, count: 0, lastEvent: null, flagNames: [], flash: false };
  s.set(zoneId, fresh);
  return fresh;
}

function onEnvelope(
  set: (cb: (s: LiveState) => Partial<LiveState>) => void,
  env: WsEnvelope,
) {
  set((s) => {
    const base = { lastMessageAt: Date.now() } as Partial<LiveState>;
    switch (env.kind) {
      case "count_event": {
        const e = env.payload as CountEvent;
        if (!dedupe.accept(e.event_id)) return base;
        const counts = new Map(s.counts);
        const z = zoneOf(counts, e.zone_id);
        counts.set(e.zone_id, {
          ...z,
          count: z.count + e.count_delta,
          lastEvent: e,
          flagNames: [],
          flash: true,
        });
        return { ...base, counts };
      }
      case "fusion_signal": {
        const f = env.payload as FusionSignal;
        return { ...base, fusion: new Map(s.fusion).set(f.zone_id, f) };
      }
      case "rl_transition":
        return { ...base, rl: env.payload as RlTransition };
      case "lifecycle_event":
        return { ...base, lifecycle: env.payload as LifecycleEvent };
      case "camera_health": {
        const h = env.payload as CameraHealth;
        return { ...base, health: new Map(s.health).set(h.camera_id, h) };
      }
      case "label_queue_added": {
        const l = env.payload as { frame_id: string; ts: string; zone_id: string };
        return { ...base, labels: [l, ...s.labels].slice(0, 100) };
      }
      default:
        return base;
    }
  });
}

interface LiveApi {
  store: { getState: () => LiveState; setState: (fn: (s: LiveState) => LiveState) => void };
  connect: (url: string) => void;
  close: () => void;
}

export interface UseLiveStore {
  <S>(selector: (s: LiveState) => S): S;
  api: LiveApi;
}

const useLiveBase = create<LiveState>(() => ({
  connected: false,
  attempts: 0,
  lastMessageAt: 0,
  now: 0,
  counts: new Map(),
  fusion: new Map(),
  rl: null,
  lifecycle: null,
  health: new Map(),
  labels: [],
  peek(zoneId) {
    return zoneOf(this.counts, zoneId);
  },
  stale() {
    return this.lastMessageAt !== 0 && Date.now() - this.lastMessageAt > STALE_AFTER_MS;
  },
}));

let socket: WebSocket | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;

async function openLive(path: string, attempt: number) {
  if (socket) return;
  const proto =typeof window !== "undefined" && window.location.protocol === "https:" ? "wss" : "ws";
  const abs = path.startsWith("ws") ? path : `${proto}://${window.location.host}${path}`;
  try {
    const sock = new WebSocket(abs);
    socket = sock;
    sock.onopen = () =>
      useLiveBase.setState((s: LiveState) => ({ ...s, connected: true, attempts: 0 }));
    sock.onmessage = (msg) => {
      try {
        const env = JSON.parse(String(msg.data)) as WsEnvelope;
        onEnvelope((cb) => useLiveBase.setState(cb), env);
      } catch (error) {
        void error; // malformed frame: ignore (docs/10 §3 pump guarantees shapes)
      }
    };
    sock.onclose = () => {
      socket = null;
      useLiveBase.setState((s: LiveState) => ({
        ...s,
        connected: false,
        attempts: attempt + 1,
      }));
      timer = setTimeout(() => void openLive(path, attempt + 1), backoffMs(attempt));
    };
    sock.onerror = () => sock.close();
  } catch (err) {
    void err;
    useLiveBase.setState((s: LiveState) => ({ ...s, attempts: attempt + 1 }));
    timer = setTimeout(() => void openLive(path, attempt + 1), backoffMs(attempt));
  }
}

function closeLive() {
  if (timer) clearTimeout(timer);
  timer = null;
  socket?.close();
  socket = null;
}

const staleTicker = () =>
  useLiveBase.setState((s: LiveState) => ({ ...s, now: Date.now() }));
if (typeof window !== "undefined") setInterval(staleTicker, 1000);

export const useLive = useLiveBase as unknown as UseLiveStore;
useLive.api = {
  store: {
    getState: () => useLiveBase.getState(),
    setState: (fn) => useLiveBase.setState(fn),
  },
  connect: (url) => void openLive(url, 0),
  close: closeLive,
};

/**
 * Fetch a REST endpoint relative to the browser origin (Next rewrites proxy
 * to the Rust backend; see next.config.ts).
 */
export async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!res.ok) {
    let detail = `${res.status}`;
    try {
      detail = await res.text();
    } catch {
      /* body unreadable */
    }
    throw new Error(`API ${path}: ${detail}`);
  }
  return (await res.json()) as T;
}
