"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiFetch } from "@/lib/live";
import type { PendingCorrectionRow } from "@/lib/types";
import CorrectionForm from "@/components/CorrectionForm";
import CorrectionCommitQueue from "@/components/CorrectionCommitQueue";

/** Corrections desk (docs/12 §2 /audit, grounded on the docs/10 wire):
 *  submit new corrections + commit the pending ones (reward lands at commit). */
export default function AuditPage() {
  const queryClient = useQueryClient();
  const pending = useQuery({
    queryKey: ["pending"],
    queryFn: () =>
      apiFetch<{ pending: PendingCorrectionRow[] }>("/api/v1/audit/pending"),
    retry: 1,
    refetchInterval: 30_000,
  });

  const remove = (correctionId: string) => {
    queryClient.setQueryData<{ pending: PendingCorrectionRow[] }>(["pending"], (prev) =>
      prev
        ? { pending: prev.pending.filter((r) => r.correction_id !== correctionId) }
        : prev,
    );
  };

  const rows = (pending.data?.pending ?? []).filter((r) => r.outcome === "submitted");

  return (
    <div className="space-y-4" data-testid="audit-page">
      <h1 className="text-lg font-semibold text-zinc-100">Corrections</h1>
      <CorrectionForm />
      {pending.isLoading && (
        <p data-testid="pending-loading" className="text-sm text-zinc-500">
          loading pending commits…
        </p>
      )}
      {pending.error && (
        <p data-testid="audit-offline" className="text-sm text-red-400">
          backend unreachable ({String(pending.error).match(/\d+/)?.[0] ?? "?"}) — start
          the Rust server to fetch pending commits
        </p>
      )}
      <CorrectionCommitQueue rows={rows} onDone={remove} />
      {pending.data && rows.length === 0 && (
        <p data-testid="audit-clear" className="text-sm text-lime-300">
          no pending commits — all clear
        </p>
      )}
    </div>
  );
}
