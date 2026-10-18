"use client";

import type { ZoneLive } from "@/lib/live";
import ConfBadge from "./ConfBadge";
import { parseUncertainty } from "@/lib/formats";
import MiniVideoTile from "./MiniVideoTile";

const FLAG_CHIP: Record<string, string> = {
  OCCLUDED_ESTIMATE: "bg-sky-700 text-sky-100",
  MODALITY_DIVERGENCE: "bg-fuchsia-700 text-fuchsia-100",
  CLOCK_DEGRADED: "bg-orange-700 text-orange-100",
  LOW_TRACK_STRENGTH: "bg-zinc-600 text-zinc-100",
};

export default function ZoneCard({
  live,
  cameraId,
}: {
  live: ZoneLive;
  /** Camera wiring for this zone comes from the venue config (docs/13);
   *  the dashboard passes it only when the venue template provides one. */
  cameraId?: string;
}) {
  const e = live.lastEvent;
  const flags = [
    ...new Set([...live.flagNames, ...parseUncertainty(e?.uncertainty ?? "")]),
  ];
  return (
    <section
      data-testid="zone-card"
      className={`rounded-lg border border-zinc-700 bg-zinc-900 p-4 ${
        live.flash ? "animate-pulse-step" : ""
      }`}
    >
      <header className="flex items-baseline justify-between">
        <h3 className="text-sm font-semibold text-zinc-200">{live.zoneId}</h3>
        {e && <ConfBadge value={e.confidence} />}
      </header>
      {cameraId && <MiniVideoTile cameraId={cameraId} />}
      <p data-testid="zone-count" className="text-3xl font-semibold text-zinc-50 tabular-nums">
        {live.count}
      </p>
      <footer className="mt-2 flex flex-wrap gap-1">
        {flags.map((f) => (
          <span
            key={f}
            className={`rounded px-1.5 py-0.5 text-[11px] font-medium ${FLAG_CHIP[f] ?? "bg-zinc-700 text-zinc-200"}`}
          >
            {f.toLowerCase().replace(/_/g, " ")}
          </span>
        ))}
        {e && (
          <span className="font-mono text-[11px] text-zinc-400">
            τ {e.thresholds_used.conf_threshold.toFixed(2)} · blend v
            {e.modality_blend.vision.toFixed(2)}/w{e.modality_blend.wifi.toFixed(2)}
          </span>
        )}
      </footer>
    </section>
  );
}
