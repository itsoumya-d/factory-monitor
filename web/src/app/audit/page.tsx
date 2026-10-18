"use client";

import { useQuery } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import type { PendingWindow } from "@/lib/types";
import CorrectionForm from "@/components/CorrectionForm";

/** Corrections tab (docs/12 §2 /audit): pending windows + the form. */
export default function AuditPage() {
  const { data, error, isLoading } = useQuery({
    queryKey: ["pending"],
    queryFn: () => apiFetch<{ pending: PendingWindow[] }>("/api/v1/audit/pending"),
    retry: 1,
  });

  return (
    <div className="space-y-4" data-testid="audit-page">
      <h1 className="text-lg font-semibold text-zinc-100">Corrections</h1>
      {isLoading && <p className="text-sm text-zinc-500">loading pending windows…</p>}
      {error && (
        <p data-testid="audit-offline" className="text-sm text-red-400">
          backend unreachable ({String(error).match(/\d+/)?.[0] ?? "?"}) — start the
          Rust server to fetch live windows
        </p>
      )}
      {data && <CorrectionForm windows={data.pending ?? []} />}
      {data && data.pending.length === 0 && (
        <p data-testid="audit-clear" className="text-sm text-lime-300">
          no windows need review — all clear
        </p>
      )}
    </div>
  );
}
