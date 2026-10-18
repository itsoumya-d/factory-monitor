/**
 * Series shaping for the EOD report (docs/12 §5): hourly rollup rows from
 * GET /api/v1/zones/{zone_id}/hourly into chart series + goal line.
 */

export interface HourlyRow {
  hour: string;
  zone_id: string;
  delta: number;
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
