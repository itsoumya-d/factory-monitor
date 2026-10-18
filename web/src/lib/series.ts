/**
 * Series shaping for the EOD report (docs/12 §5): hourly rollup rows from
 * GET /api/v1/zones/{zone_id}/hourly into chart series + goal line.
 */

export interface HourlyRow {
  hour: string;
  zone_id: string;
  delta: number;
  /** zone-day avg_conf as the backend hourly rollup carries it (docs/10). */
  avg_conf?: number;
}

/** Hour bucket exactly as the Rust agg module serializes it (docs/10). */
export interface HourlyPoint {
  hour: string;
  packages: number;
  avg_conf?: number;
  occluded_events?: number;
}

/** GET /api/v1/zones/{z}/hourly response body. */
export interface HourlyResponse {
  zone_id: string;
  date: string;
  hours: HourlyPoint[];
}

/** Flatten one zone's server hours into universal rows ("14" stays "14"). */
export function fromHourlyPoints(zoneId: string, pts: HourlyPoint[]): HourlyRow[] {
  return pts.map((p) => ({
    zone_id: zoneId,
    hour: p.hour.slice(11, 13),
    delta: p.packages,
    avg_conf: p.avg_conf,
  }));
}

export interface SeriesPoint {
  hour: string;
  [zone: string]: number | string;
}

/** Sum rows into one point per hour; missing hours interpolant-padded to zero. */
export function hourlySeries(rows: HourlyRow[], totalHours = 24): SeriesPoint[] {
  const zones = [...new Set(rows.map((r) => r.zone_id))].sort();
  const buckets = new Map<string, SeriesPoint>();
  for (let h = 0; h < totalHours; h++) {
    const label = `${String(h).padStart(2, "0")}`;
    const point: SeriesPoint = { hour: label };
    for (const z of zones) point[zoneKey(z)] = 0;
    buckets.set(label, point);
  }
  for (const r of rows) {
    const label = String(r.hour).padStart(2, "0");
    const point = buckets.get(label);
    if (point) point[zoneKey(r.zone_id)] = (point[zoneKey(r.zone_id)] as number) + r.delta;
  }
  return [...buckets.values()];
}

/** Recharts forbids dots/invalid property keys: zone ids become series keys. */
export function zoneKey(zoneId: string): string {
  return zoneId.replace(/[^A-Za-z0-9_]/g, "_");
}

export function zoneList(rows: HourlyRow[]): string[] {
  return [...new Set(rows.map((r) => r.zone_id))].sort();
}

/** Daily goal driven from the docs (e.g. 25% of the last good day). */
export function goalPerHour(goalTotal: number): number {
  return Math.round(goalTotal / 24);
}

/** docs/12 §5 run 4: LOW_CONFIDENCE spans = worst hourly rollups per zone. */
export interface ReviewWindow {
  zone_id: string;
  hour: string;
  avg_conf: number;
}

/** Hours whose zone avg_conf falls below the review threshold. */
export function lowConfidenceWindows(rows: HourlyRow[], threshold = 0.75): ReviewWindow[] {
  const windows: ReviewWindow[] = [];
  for (const r of rows) {
    if (r.avg_conf !== undefined && r.avg_conf < threshold) {
      windows.push({ zone_id: r.zone_id, hour: r.hour, avg_conf: r.avg_conf });
    }
  }
  return windows.sort((a, b) => (a.zone_id + a.hour).localeCompare(b.zone_id + b.hour));
}

/** docs/12 §5 run 5: the CSV export (hourly rollup, one row per zone-hour). */
export function reportCsv(rows: HourlyRow[], date: string): string {
  const head = `zone_id,hour,packages,avg_conf,date`;
  const lines = rows.map((r) => `${r.zone_id},${r.hour},${r.delta},${r.avg_conf?.toFixed(3) ?? ""},${date}`);
  return [head, ...lines].join("\n");
}
