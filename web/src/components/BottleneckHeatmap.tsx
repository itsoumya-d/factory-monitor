/**
 * Bottleneck heatmap (docs/12 §5): zone × hour grid where every cell is the
 * hourly package delta from GET /api/v1/zones/{z}/hourly. Pure DOM — no
 * canvas — so screen readers and tests see the same numbers as operators.
 */

import type { HourlyRow } from "@/lib/series";
import { zoneKey, zoneList } from "@/lib/series";

export type Ramp = "calm" | "rising" | "busy" | "storm";

/** Discrete calm→storm ramp: share of the day's busiest hour (docs/12 §5). */
export function heatRamp(delta: number, max: number): Ramp {
  if (max <= 0 || delta <= 0) return "calm";
  const share = delta / max;
  if (share > 0.75) return "storm";
  if (share > 0.5) return "busy";
  if (share > 0.25) return "rising";
  return "calm";
}

const BG: Record<Ramp, string> = {
  calm: "bg-zinc-900 text-zinc-500",
  rising: "bg-sky-900 text-sky-100",
  busy: "bg-amber-500 text-zinc-950",
  storm: "bg-red-600 text-white",
};

interface Props {
  rows: HourlyRow[];
  holiday?: boolean;
}

/** Dense grid: one row per zone, 24 monospace hour cells with delta. */
export default function BottleneckHeatmap({ rows, holiday = false }: Props) {
  const zones = zoneList(rows);
  const max = rows.reduce((m, r) => Math.max(m, r.delta), 0);
  const cell = new Map<string, number>();
  for (const r of rows) cell.set(`${r.zone_id}@${r.hour}`, r.delta);

  const top = rows.filter((r) => heatRamp(r.delta, max) === "storm");
  const busiest = top.length > 0 ? top[top.length - 1] : null;

  return (
    <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
      <h2 className="text-sm font-semibold text-zinc-200">Bottleneck heatmap</h2>
      {holiday && (
        <p className="text-xs font-semibold text-amber-400" data-testid="heatmap-holiday">
          holiday schedule — compare against the same weekday, not the trailing week
        </p>
      )}
      <p className="text-xs text-zinc-400">
        hourly package deltas, darker/amber/red = closer to {max} pkgs peak
        <span className="ml-2 inline-flex items-center gap-1" aria-hidden>
          <i className="inline-block h-3 w-3 rounded bg-zinc-900" />
          <i className="inline-block h-3 w-3 rounded bg-sky-900" />
          <i className="inline-block h-3 w-3 rounded bg-amber-500" />
          <i className="inline-block h-3 w-3 rounded bg-red-600" />
          <span className="ml-1 text-zinc-300">calm → storm</span>
        </span>
      </p>
      <div className="mt-3 overflow-x-auto">
        <table className="border-separate border-spacing-0.5 font-mono text-[10px]">
          <tbody data-testid="heatmap-body" role="img" aria-label={`zone by hour heatmap, peak ${max}`}>
            {zones.map((z) => (
              <tr key={z}>
                <th className="pr-1 text-right text-zinc-300">{zoneKey(z)}</th>
                {Array.from({ length: 24 }, (_, h) => {
                  const label = String(h).padStart(2, "0");
                  const d = cell.get(`${z}@${label}`) ?? 0;
                  const ramp = heatRamp(d, max);
                  return (
                    <td
                      key={label}
                      data-testid={`heat-${zoneKey(z)}-${label}`}
                      data-ramp={ramp}
                      data-delta={d}
                      className={`h-4 min-w-6 rounded text-center ${BG[ramp]}`}
                      title={`${z} ${label}:00 — ${d} pkgs`}
                    >
                      {d > 0 ? d : ""}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {busiest && (
        <p className="mt-2 text-xs text-zinc-400" data-testid="heatmap-busiest">
          hottest window: {zoneKey(busiest.zone_id)} at {busiest.hour}:00 ({busiest.delta} pkgs)
        </p>
      )}
    </section>
  );
}
