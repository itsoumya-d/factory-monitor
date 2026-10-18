"use client";

import { useQuery } from "@tanstack/react-query";
import { apiFetch, useLive } from "@/lib/live";
import RoiEditor from "@/components/RoiEditor";
import type { PendingWindow } from "@/lib/types";

/** ROI editor (docs/12 §2 /settings/rois): zones from the live feed, with
 * a one-shot fallback to /api/v1/audit/pending zone ids. */
export default function RoisPage() {
  const liveZones = useLive((s) => [...s.counts.keys()]);
  const { data } = useQuery({
    queryKey: ["pending-zones"],
    queryFn: () => apiFetch<{ pending: PendingWindow[] }>("/api/v1/audit/pending"),
    retry: 1,
    staleTime: 60_000,
  });
  const fallbackZones = [...new Set((data?.pending ?? []).map((w) => w.zone_id))];
  const zones = [...new Set([...liveZones, ...fallbackZones])];

  return (
    <div className="space-y-4" data-testid="rois-page">
      <h1 className="text-lg font-semibold text-zinc-100">ROI editor</h1>
      <p className="text-sm text-zinc-400">
        Click to place counting-line points, End line to finish a rope, click
        near a finished rope&apos;s end to pop it. Legal lines post to{" "}
        <code className="text-zinc-300">/api/v1/calibration/rois/&#123;zone_id&#125;</code>.
      </p>
      {zones.length === 0 ? (
        <p data-testid="roi-no-zones" className="text-sm text-zinc-500">
          no zones seen yet — start the live feed or the backend
        </p>
      ) : (
        <RoiEditor zones={zones} />
      )}
    </div>
  );
}
