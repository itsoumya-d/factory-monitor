"use client";

import { useEffect, useMemo, useState } from "react";
import { useLive } from "@/lib/live";

/** Ticker of live cross events (CounterTicker docs/12 §3). */
export default function CounterTicker() {
  // Select the Map ref (stable until the next store write); derive with
  // useMemo — in-selector array spreads re-render-loop in React 19.
  const countsMap = useLive((s) => s.counts);
  const counts = useMemo(() => [...countsMap.values()], [countsMap]);
  const [flashes, setFlashes] = useState<string[]>([]);

  // Down-convert flashes (<600ms per photosensitivity rule).
  useEffect(() => {
    const hot = counts.filter((z) => z.flash).map((z) => z.zoneId);
    if (!hot.length) return;
    setFlashes(hot);
    const t = setTimeout(() => setFlashes([]), 550);
    return () => clearTimeout(t);
  }, [counts]);

  const rows = counts.filter((z) => z.lastEvent);
  if (!rows.length) {
    return (
      <p data-testid="ticker-empty" className="text-sm text-zinc-500">
        Waiting for live crossings…
      </p>
    );
  }
  return (
    <ol data-testid="ticker" className="space-y-1 font-mono text-sm text-zinc-200">
      {rows.map((z) => (
        <li key={z.zoneId} className="flex justify-between">
          <span>{z.zoneId}</span>
          <strong className={flashes.includes(z.zoneId) ? "text-lime-300" : ""}>
            +{z.lastEvent?.count_delta ?? 0}
          </strong>
        </li>
      ))}
    </ol>
  );
}
