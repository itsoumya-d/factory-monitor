"use client";

import { useState } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import LiveThroughputChart from "@/components/LiveThroughputChart";
import BottleneckHeatmap from "@/components/BottleneckHeatmap";
import SuspectWindowViewer from "@/components/SuspectWindowViewer";
import type { HourlyResponse, HourlyRow } from "@/lib/series";
import { fromHourlyPoints, lowConfidenceWindows, reportCsv } from "@/lib/series";
import type { PendingCorrectionRow } from "@/lib/types";

/** Hourly rollups from the Rust counting API (docs/10): one fetch per zone,
 *  zones discovered from submitted corrections awaiting commit. */
export default function ReportFor({ date }: { date: string }) {
  const valid = /^\d{4}-\d{2}-\d{2}$/.test(date);
  const [viewer, setViewer] = useState<{ zone: string; hour: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [csvNote, setCsvNote] = useState<string | null>(null);

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
  const windows = lowConfidenceWindows(rows);

  const exportCsv = () => {
    try {
      const blob = new Blob([reportCsv(rows, date)], { type: "text/csv" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `shado-report-${date}.csv`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      setCsvNote("csv downloaded");
    } catch (e) {
      setCsvNote(`csv failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const copyWebhook = async () => {
    const payload = JSON.stringify({ date, zones, rows });
    try {
      if (!navigator.clipboard) throw new TypeError("clipboard unavailable");
      await navigator.clipboard.writeText(payload);
      setCopied(true);
    } catch {
      setCsvNote("copy blocked by browser permissions");
    }
  };

  if (!valid) {
    return <p className="text-sm text-red-400">bad date format (needs YYYY-MM-DD)</p>;
  }

  const loading = pendingQ.isLoading || (zones.length > 0 && hourlies.some((q) => q.isLoading));
  return (
    <div className="space-y-3" data-testid="report-blocks">
      <LiveThroughputChart rows={rows} />
      <BottleneckHeatmap rows={rows} />
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4" data-testid="windows-to-review">
        <h2 className="text-sm font-semibold text-zinc-200">windows to review</h2>
        {loading ? (
          <p className="text-sm text-zinc-500">loading windows…</p>
        ) : windows.length === 0 ? (
          <p className="text-sm text-zinc-400">
            no LOW_CONFIDENCE hour spans — every zone avg_conf stayed above 0.75
          </p>
        ) : (
          <ul className="mt-1 space-y-1 font-mono text-sm text-zinc-300">
            {windows.map((w) => (
              <li key={`${w.zone_id}-${w.hour}`}>
                <button
                  type="button"
                  onClick={() => setViewer({ zone: w.zone_id, hour: w.hour })}
                  data-testid={`window-${w.zone_id}-${w.hour}`}
                  className="rounded bg-zinc-800 px-2 py-1 text-xs text-amber-300"
                >
                  {w.zone_id} · {w.hour}:00 · avg_conf {w.avg_conf.toFixed(2)} → open viewer
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
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
                <td className="py-1 text-right">{r.avg_conf?.toFixed(2) ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <section className="rounded border border-zinc-800 bg-zinc-900 p-4">
        <h2 className="text-sm font-semibold text-zinc-200">export</h2>
        <div className="mt-2 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={exportCsv}
            className="rounded bg-zinc-800 px-3 py-1.5 text-xs text-zinc-300"
            data-testid="export-csv"
          >
            export CSV
          </button>
          <button
            type="button"
            onClick={() => window.print()}
            className="rounded bg-zinc-800 px-3 py-1.5 text-xs text-zinc-300"
            data-testid="export-pdf"
          >
            export PDF (print)
          </button>
          <button
            type="button"
            onClick={() => void copyWebhook()}
            className="rounded bg-zinc-800 px-3 py-1.5 text-xs text-zinc-300"
            data-testid="copy-webhook"
          >
            {copied ? "webhook payload copied ✓" : "copy webhook payload"}
          </button>
          {csvNote && (
            <span className="text-xs text-zinc-400" data-testid="export-note">
              {csvNote}
            </span>
          )}
        </div>
      </section>
      {viewer && (
        <SuspectWindowViewer
          zoneId={viewer.zone}
          ts={`${date}T${viewer.hour}:00:00Z`}
          onClose={() => setViewer(null)}
        />
      )}
    </div>
  );
}
