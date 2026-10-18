"use client";

import { useLive } from "@/lib/live";

const STATES = ["Standby", "Calibrating", "Counting", "Correcting", "Degraded"] as const;
const DOT: Record<string, string> = {
  Standby: "bg-zinc-500",
  Calibrating: "bg-sky-400",
  Counting: "bg-emerald-400",
  Correcting: "bg-amber-400",
  Degraded: "bg-red-500",
};

export default function LifecycleStrip() {
  const lifecycle = useLive((s) => s.lifecycle);
  const current = lifecycle?.to ?? "Standby";
  return (
    <div
      data-testid="lifecycle-strip"
      className="flex gap-3 rounded bg-zinc-900 px-3 py-2 border border-zinc-800"
      aria-label={`lifecycle ${current}`}
    >
      {STATES.map((st) => (
        <span key={st} className="flex items-center gap-1.5 text-xs">
          <span
            className={`inline-block h-2.5 w-2.5 rounded-full ${DOT[st]} ${st === current ? "opacity-100" : "opacity-25"}`}
          />
          <span className={st === current ? "text-zinc-100 font-medium" : "text-zinc-500"}>
            {st}
          </span>
        </span>
      ))}
      {current === "Degraded" && (
        <span className="text-xs text-red-300">check camera + CSI health</span>
      )}
    </div>
  );
}
