"use client";

import { useEffect, useMemo } from "react";
import { useLive } from "@/lib/live";
import ZoneCard from "@/components/ZoneCard";
import CounterTicker from "@/components/CounterTicker";
import LifecycleStrip from "@/components/LifecycleStrip";

/** Live ops (docs/12 §2 /dashboard): zone cards, ticker, health strip. */
export default function DashboardPage() {
  const countsMap = useLive((s) => s.counts);
  const healthMap = useLive((s) => s.health);
  const zones = useMemo(() => [...countsMap.values()], [countsMap]);
  const health = useMemo(() => [...healthMap.values()], [healthMap]);

  // Demo zones appear the first time the feed opens.
  useEffect(() => {
    if (zones.length === 0) {
      useLive.api.store.getState().peek("line_1");
      useLive.api.store.getState().peek("line_2");
      useLive.api.store.getState().peek("dock_gate");
    }
  }, [zones.length]);

  return (
    <div className="space-y-4">
      <LifecycleStrip />
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {zones.map((z) => (
          <ZoneCard live={z} key={z.zoneId} />
        ))}
      </div>
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h2 className="mb-2 text-sm font-semibold text-zinc-200">Crossing ticker</h2>
        <CounterTicker />
      </section>
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h2 className="mb-2 text-sm font-semibold text-zinc-200">Camera health</h2>
        {health.length === 0 ? (
          <p data-testid="health-empty" className="text-sm text-zinc-500">
            no camera reports yet
          </p>
        ) : (
          <ul className="font-mono text-sm text-zinc-200">
            {health.map((h) => (
              <li key={h.camera_id}>
                {h.camera_id}: {h.ok ? "ok" : "down"} · {h.fps.toFixed(1)} fps · drop{" "}
                {(h.dropped_ratio * 100).toFixed(0)}%
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
