"use client";

import { useQueries, useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import LiveThroughputChart from "@/components/LiveThroughputChart";
import BottleneckHeatmap from "@/components/BottleneckHeatmap";
import type { HourlyResponse, HourlyRow } from "@/lib/series";
import { fromHourlyPoints } from "@/lib/series";
import type { PendingCorrectionRow } from "@/lib/types";

/** Hourly rollups from the Rust counting API (docs/10): one fetch per zone,
 *  zones discovered from submitted corrections awaiting commit. */
export default function ReportFor({ date }: { date: string }) {
  const valid = /^\d{4}-\d{2}-\d{2}$/.test(date);

  const pendingQ = useQuery({
    queryKey: ["audit-pending"],
    queryFn: () =>
      apiFetch<{ pending: PendingCorrectionRow[] }>("/api/v1/audit/pending"),
    enabled: valid,
  });
  const zones = [
    ...new Set((pendingQ.data?.pending ?? []).map((r) => r.feedback.zone_id)),
  ].sort();

  const hourlies = useQueries({
    queries: zones.map((z) => ({
      queryKey: ["hourly", z, date],
      queryFn: () =>
        apiFetch<HourlyResponse>(`/api/v1/zones/${encodeURIComponent(z)}/hourly?date=${date}`),
      enabled: valid && zones.length > 0,
    })),
  });

  const rows: HourlyRow[] = hourlies.flatMap((q, i) =>
    q.data ? fromHourlyPoints(zones[i], q.data.hours ?? []) : [],
  );

  if (!valid) {
    return <p className="text-sm text-red-400">bad date format (needs YYYY-MM-DD)</p>;
  }

  const loading = pendingQ.isLoading || (zones.length > 0 && hourlies.some((q) => q.isLoading));
  return (
    <div className="space-y-3" data-testid="report-blocks">
      <LiveThroughputChart rows={rows} />
      <BottleneckHeatmap rows={rows} />
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h2 className="text-sm font-semibold text-zinc-200">Hourly rollup</h2>
        <p className="text-sm text-zinc-400">
          {loading
            ? "loading hourly rollups…"
            : zones.length === 0
              ? "no zones with pending windows yet — the report fills once corrections flow"
              : `${zones.length} zone(s) × 24h tallied`}
        </p>
        <table className="mt-2 w-full font-mono text-sm text-zinc-300">
          <tbody data-testid="hourly-table">
            {rows.map((r, i) => (
              <tr key={`${r.zone_id}-${r.hour}-${i}`} className="border-t border-zinc-800">
                <td className="py-1">{r.zone_id}</td>
                <td className="py-1">{r.hour}:00</td>
                <td className="py-1 text-right">{r.delta}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}
