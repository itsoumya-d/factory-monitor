import { confidenceLevel, type ConfLevel } from "@/lib/formats";

const CHIP: Record<ConfLevel, string> = {
  green: "bg-emerald-600/90 text-emerald-50",
  amber: "bg-amber-500/90 text-amber-950",
  red: "bg-red-600/90 text-red-50",
};

export default function ConfBadge({
  value,
  level,
}: {
  value?: number;
  level?: ConfLevel;
}) {
  const lv = level ?? confidenceLevel(value ?? 0);
  const pct = Math.round((value ?? 0) * 100);
  const label = typeof value === "number" ? `${pct}` : lv;
  return (
    <span
      data-testid={`conf-${lv}`}
      className={`inline-flex h-7 min-w-12 items-center justify-center rounded px-2 font-mono text-xs tabular-nums ${CHIP[lv]}`}
      aria-label={`confidence ${pct}% ${lv}`}
    >
      {label}
    </span>
  );
}
