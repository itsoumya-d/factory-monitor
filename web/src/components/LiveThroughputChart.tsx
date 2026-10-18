"use client";

import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { goalPerHour, hourlySeries, zoneKey, zoneList } from "@/lib/series";
import type { HourlyRow } from "@/lib/series";

const PALETTE = ["#a3e635", "#38bdf8", "#f472b6", "#fb923c", "#c4b5fd"];

/**
 * LiveThroughputChart (docs/12 §3): hourly AreaChart + goal line.
 * Numbers stay right-aligned in the tooltip; one series per zone.
 */
export default function LiveThroughputChart({
  rows,
  goalTotal,
}: {
  rows: HourlyRow[];
  goalTotal?: number;
}) {
  const data = hourlySeries(rows);
  const zones = zoneList(rows);
  const goal = goalTotal !== undefined ? goalPerHour(goalTotal) : undefined;

  return (
    <section className="rounded border border-zinc-800 bg-zinc-900 p-4" data-testid="throughput-chart">
      <h2 className="mb-2 text-sm font-semibold text-zinc-200">Hourly throughput</h2>
      <div style={{ width: "100%", height: 260 }}>
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={data} margin={{ top: 6, right: 12, bottom: 6, left: 0 }}>
            <CartesianGrid stroke="#3f3f46" strokeDasharray="3 3" />
            <XAxis dataKey="hour" tick={{ fill: "#a1a1aa", fontSize: 11 }} stroke="#52525b" />
            <YAxis tick={{ fill: "#a1a1aa", fontSize: 11 }} stroke="#52525b" />
            <Tooltip
              cursor={{ stroke: "#52525b" }}
              contentStyle={{
                background: "#18181b",
                border: "1px solid #3f3f46",
                fontFamily: "var(--font-mono)",
                textAlign: "right",
              }}
            />
            {zones.length > 1 && <Legend />}
            {goal !== undefined && (
              <ReferenceLine y={goal} stroke="#eab308" strokeDasharray="6 4" />
            )}
            {zones.map((z, i) => (
              <Area
                key={z}
                dataKey={zoneKey(z)}
                name={z}
                type="monotone"
                stroke={PALETTE[i % PALETTE.length]}
                fill={PALETTE[i % PALETTE.length]}
                fillOpacity={0.18}
                strokeWidth={2}
                isAnimationActive={false}
              />
            ))}
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </section>
  );
}
