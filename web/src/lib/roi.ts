/**
 * ROI geometry, mirroring monitoring_contract::geometry and
 * inference_engine::zones::segments_intersect exactly (GeoPoint:
 * normalized [0..1] inclusive and finite; the REST body for
 * POST /api/v1/calibration/rois/{zone_id} is {"lines": [[[x,y], ...], ...]}
 * with ≥1 line and ≥2 points per line).
 */

export type GeoPoint = { x: number; y: number };
export type RoiLine = { points: GeoPoint[] };

export const EMPTY_LINES = "lines must be non-empty";
export const SHORT_LINE = "a counting line needs at least two points";
export const OUT_OF_BOUNDS = "points must be finite in [0,1]";

/** GeoPoint::finite_and_bounded equivalent. */
export function finiteAndBounded(p: GeoPoint): boolean {
  return (
    Number.isFinite(p.x) &&
    Number.isFinite(p.y) &&
    p.x >= 0 &&
    p.x <= 1 &&
    p.y >= 0 &&
    p.y <= 1
  );
}

/** Lines validated point-wise: ≥2 points, all finite and bounded. */
export function validateRoiLines(lines: RoiLine[]): string | null {
  if (lines.length === 0) return EMPTY_LINES;
  for (const l of lines) {
    if (l.points.length < 2) return SHORT_LINE;
    if (!l.points.every(finiteAndBounded)) return OUT_OF_BOUNDS;
  }
  return null;
}

/** Encode the editor's lines into the exact REST body (throws on invalid). */
export function roiSubmitBody(lines: RoiLine[]): { lines: GeoPoint[][] } {
  const error = validateRoiLines(lines);
  if (error) throw new Error(error);
  return { lines: lines.map((l) => l.points) };
}

/** Squared distance from point p to segment ab (canvas hit tests). */
export function distPointToSegmentSq(p: GeoPoint, a: GeoPoint, b: GeoPoint): number {
  const abx = b.x - a.x;
  const aby = b.y - a.y;
  const apx = p.x - a.x;
  const apy = p.y - a.y;
  const lenSq = abx * abx + aby * aby;
  const t = lenSq === 0 ? 0 : Math.max(0, Math.min(1, (apx * abx + apy * aby) / lenSq));
  const dx = apx - t * abx;
  const dy = apy - t * aby;
  return dx * dx + dy * dy;
}

/** Faithful port of `inference_engine::zones::segments_intersect`. */
export function segmentsIntersect(p1: GeoPoint, p2: GeoPoint, q1: GeoPoint, q2: GeoPoint): boolean {
  const orient = (a: GeoPoint, b: GeoPoint, p: GeoPoint): number =>
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
  const onSegment = (a: GeoPoint, b: GeoPoint, p: GeoPoint): boolean => {
    const minX = Math.min(a.x, b.x);
    const maxX = Math.max(a.x, b.x);
    const minY = Math.min(a.y, b.y);
    const maxY = Math.max(a.y, b.y);
    return p.x >= minX - 1e-6 && p.x <= maxX + 1e-6 && p.y >= minY - 1e-6 && p.y <= maxY + 1e-6;
  };
  const d1 = orient(q1, q2, p1);
  const d2 = orient(q1, q2, p2);
  const d3 = orient(p1, p2, q1);
  const d4 = orient(p1, p2, q2);

  if (((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))) {
    return true;
  }
  const zero = 1e-6;
  if (Math.abs(d1) < zero && onSegment(q1, q2, p1)) return true;
  if (Math.abs(d2) < zero && onSegment(q1, q2, p2)) return true;
  if (Math.abs(d3) < zero && onSegment(p1, p2, q1)) return true;
  if (Math.abs(d4) < zero && onSegment(p1, p2, q2)) return true;
  return false;
}
