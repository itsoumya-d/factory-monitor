"use client";

import { useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import LiveThroughputChart from "@/components/LiveThroughputChart";
import type { HourlyRow } from "@/lib/series";

/** Hourly rollup from the Rust counting API (GET /api/v1/zones/{z}/hourly). */
export default function ReportFor({ date }: { date: string }) {
  const valid = /^\d{4}-\d{2}-\d{2}$/.test(date);
  const { data } = useQuery({
    queryKey: ["hourly", date],
    queryFn: () => apiFetch<{ rows: HourlyRow[] }>(`/api/v1/zones/-/hourly?date=${date}`),
    enabled: valid,
  });
  if (!valid) return <p className="text-sm text-red-400">bad date format</p>;

  const rows = data?.rows ?? [];
  return (
    <div className="space-y-3" data-testid="report-blocks">
      <LiveThroughputChart rows={rows} />
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h2 className="text-sm font-semibold text-zinc-200">Hourly rollup</h2>
        <p className="text-sm text-zinc-400">
          {rows.length === 0
            ? "no hourly data for this date (server empty or route offline)"
            : `${rows.length} row(s) tallied`}
        </p>
        <table className="mt-2 w-full font-mono text-sm text-zinc-300">
          <tbody data-testid="hourly-table">
            {rows.map((r, i) => (
              <tr key={`${r.zone_id}-${r.hour}-${i}`} className="border-t border-zinc-800">
                <td className="py-1">{r.zone_id}</td>
                <td className="py-1">{r.hour}</td>
                <td className="py-1 text-right">{r.delta}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}

