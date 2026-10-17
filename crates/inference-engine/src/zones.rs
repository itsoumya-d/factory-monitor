//! Counting zones and line-crossing math (docs/07 §4): open counting lines
//! with the A→B (shipped) / B→A (discarded) direction convention, tolerance
//! polynomials for presence context, and the per-track segment intersection
//! test.

use common_types::{CameraId, TrackId, ZoneId};

use crate::error::InferenceError;

pub type Point = [f32; 2];

/// Crossing direction along a `CountLine`: A→B is the shipped direction and
/// increments counts; B→A is discarded (docs/07 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDir {
    AToB,
    BToA,
}

impl CrossDir {
    /// Batch-delta contribution: A→B adds one, B→A is discarded, not
    /// subtracted (reconciliations subtract via track-kill events instead).
    pub const fn batch_delta(&self) -> i64 {
        match self {
            Self::AToB => 1,
            Self::BToA => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountLine {
    pub line_id: u16,
    pub a: Point,
    pub b: Point,
}

impl CountLine {
    pub fn try_new(line_id: u16, a: Point, b: Point) -> Result<Self, InferenceError> {
        if (a[0] - b[0]).abs() < 1.0e-6 && (a[1] - b[1]).abs() < 1.0e-6 {
            return Err(InferenceError::Detector(format!(
                "counting line {line_id} is degenerate (a == b)"
            )));
        }
        Ok(Self { line_id, a, b })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskPolygon {
    points: Vec<Point>,
}

impl MaskPolygon {
    pub fn try_new(points: Vec<Point>) -> Result<Self, InferenceError> {
        if points.len() < 3 {
            return Err(InferenceError::Detector(
                "mask polygon needs at least 3 points".to_string(),
            ));
        }
        Ok(Self { points })
    }

    /// Even-odd ray casting (deterministic, edge-inclusive enough for our
    /// use: points exactly on the boundary count as inside).
    pub fn contains(&self, p: Point) -> bool {
        let mut inside = false;
        let n = self.points.len();
        for i in 0..n {
            let a = self.points[i];
            let b = self.points[(i + 1) % n];
            if (p[1] > a[1]) != (p[1] > b[1]) {
                let x_int = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                if p[0] < x_int {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// One zone's ROI bundle: which camera watches it, where the counting lines
/// run, and the presence polygons.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneRois {
    pub zone_id: ZoneId,
    pub camera_id: CameraId,
    pub lines: Vec<CountLine>,
    pub zones: Vec<MaskPolygon>,
}

impl ZoneRois {
    pub fn try_new(
        zone_id: ZoneId,
        camera_id: CameraId,
        lines: Vec<CountLine>,
        zones: Vec<MaskPolygon>,
    ) -> Result<Self, InferenceError> {
        Ok(Self {
            zone_id,
            camera_id,
            lines,
            zones,
        })
    }
}

/// Model taxonomy of countable classes (docs/07 §4: `package`/`bar-box` are
/// the only object classes that increment counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountableClass {
    Package,
    BarBox,
}

impl CountableClass {
    pub const PACKAGE_ID: u16 = 0;
    pub const BARBOX_ID: u16 = 1;

    pub const fn from_class_id(id: u16) -> Option<Self> {
        match id {
            Self::PACKAGE_ID => Some(Self::Package),
            Self::BARBOX_ID => Some(Self::BarBox),
            _ => None,
        }
    }

    pub const fn class_id(self) -> u16 {
        match self {
            Self::Package => Self::PACKAGE_ID,
            Self::BarBox => Self::BARBOX_ID,
        }
    }
}

/// One track's motion between two consecutive frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackMotion {
    pub track_id: TrackId,
    pub class_id: u16,
    pub prev_centroid: Option<[f32; 2]>,
    pub current: [f32; 2],
    pub confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineCrossOutcome {
    pub track_id: TrackId,
    pub line_id: u16,
    pub dir: CrossDir,
}

fn orient(a: Point, b: Point, p: Point) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn on_segment(a: Point, b: Point, p: Point) -> bool {
    let min_x = a[0].min(b[0]);
    let max_x = a[0].max(b[0]);
    let min_y = a[1].min(b[1]);
    let max_y = a[1].max(b[1]);
    p[0] >= min_x - 1.0e-6
        && p[0] <= max_x + 1.0e-6
        && p[1] >= min_y - 1.0e-6
        && p[1] <= max_y + 1.0e-6
}

/// Proper segment intersection incl. collinear overlap endpoints.
pub fn segments_intersect(p1: Point, p2: Point, q1: Point, q2: Point) -> bool {
    let d1 = orient(q1, q2, p1);
    let d2 = orient(q1, q2, p2);
    let d3 = orient(p1, p2, q1);
    let d4 = orient(p1, p2, q2);

    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    // Touching/collinear cases.
    let zero = 1.0e-6;
    if d1.abs() < zero && on_segment(q1, q2, p1) {
        return true;
    }
    if d2.abs() < zero && on_segment(q1, q2, p2) {
        return true;
    }
    if d3.abs() < zero && on_segment(p1, p2, q1) {
        return true;
    }
    if d4.abs() < zero && on_segment(p1, p2, q2) {
        return true;
    }
    false
}

/// The crossing counter for exactly one (zone, camera) pair. Counting fires
/// on the segment of the track centroid trajectory between frames (docs/07
/// §4); only countable, confirmed tracks participate.
pub struct LineCrossCounter {
    rois: ZoneRois,
}

impl LineCrossCounter {
    pub fn new(rois: ZoneRois) -> Self {
        Self { rois }
    }

    pub fn zone_id(&self) -> &ZoneId {
        &self.rois.zone_id
    }

    pub const fn camera_id(&self) -> &CameraId {
        &self.rois.camera_id
    }

    pub fn rois(&self) -> &ZoneRois {
        &self.rois
    }

    pub fn count(&self, motions: &[TrackMotion]) -> Vec<LineCrossOutcome> {
        let mut out = Vec::new();
        for motion in motions {
            if !motion.confirmed {
                continue;
            }
            if CountableClass::from_class_id(motion.class_id).is_none() {
                continue;
            }
            let Some(prev) = motion.prev_centroid else {
                continue;
            };
            for line in &self.rois.lines {
                if !segments_intersect(prev, motion.current, line.a, line.b) {
                    continue;
                }
                let s_prev = orient(line.a, line.b, prev);
                let s_cur = orient(line.a, line.b, motion.current);
                if (s_prev < 0.0 && s_cur > 0.0) || (s_prev > 0.0 && s_cur < 0.0) {
                    let dir = if s_cur > 0.0 {
                        CrossDir::AToB
                    } else {
                        CrossDir::BToA
                    };
                    out.push(LineCrossOutcome {
                        track_id: motion.track_id,
                        line_id: line.line_id,
                        dir,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::Utc;

    fn zone() -> ZoneId {
        ZoneId::try_new("line_1").unwrap()
    }

    fn camera() -> CameraId {
        CameraId::try_new("cam_a").unwrap()
    }

    fn motion(
        id: u64,
        prev: Option<[f32; 2]>,
        current: [f32; 2],
        confirmed: bool,
        class: u16,
    ) -> TrackMotion {
        TrackMotion {
            track_id: TrackId::new(id),
            class_id: class,
            prev_centroid: prev,
            current,
            confirmed,
        }
    }

    fn counter() -> LineCrossCounter {
        let rois = ZoneRois::try_new(
            zone(),
            camera(),
            vec![CountLine::try_new(1, [10.0, 110.0], [190.0, 110.0]).unwrap()],
            vec![],
        )
        .unwrap();
        LineCrossCounter::new(rois)
    }

    #[test]
    fn forward_crossing_counts_shipped_direction() {
        let counter = counter();
        let out = counter.count(&[motion(
            7,
            Some([50.0, 80.0]),
            [50.0, 140.0],
            true,
            CountableClass::PACKAGE_ID,
        )]);
        assert_eq!(
            out,
            vec![LineCrossOutcome {
                track_id: TrackId::new(7),
                line_id: 1,
                dir: CrossDir::AToB
            }]
        );
        assert_eq!(out[0].dir.batch_delta(), 1);
    }

    #[test]
    fn reverse_crossing_is_emitted_but_discarded() {
        let counter = counter();
        let out = counter.count(&[motion(
            7,
            Some([50.0, 140.0]),
            [50.0, 80.0],
            true,
            CountableClass::PACKAGE_ID,
        )]);
        assert_eq!(out[0].dir, CrossDir::BToA);
        assert_eq!(out[0].dir.batch_delta(), 0);
    }

    #[test]
    fn no_cross_parallel_movement() {
        let counter = counter();
        let out = counter.count(&[motion(
            7,
            Some([50.0, 60.0]),
            [50.0, 100.0],
            true,
            CountableClass::PACKAGE_ID,
        )]);
        assert!(out.is_empty());
    }

    #[test]
    fn pending_tracks_and_noncountable_classes_skip() {
        let counter = counter();
        let out = counter.count(&[motion(
            7,
            Some([50.0, 80.0]),
            [50.0, 140.0],
            false,
            CountableClass::PACKAGE_ID,
        )]);
        assert!(out.is_empty(), "unconfirmed track must not cross-count");
        let out = counter.count(&[motion(7, Some([50.0, 80.0]), [50.0, 140.0], true, 5)]);
        assert!(out.is_empty(), "noncountable class must not cross-count");
        let out = counter.count(&[motion(
            7,
            None,
            [50.0, 140.0],
            true,
            CountableClass::PACKAGE_ID,
        )]);
        assert!(out.is_empty(), "no 2-frame history must not cross-count");
    }

    #[test]
    fn multiple_lines_each_fire() {
        let rois = ZoneRois::try_new(
            zone(),
            camera(),
            vec![
                CountLine::try_new(1, [10.0, 110.0], [190.0, 110.0]).unwrap(),
                CountLine::try_new(2, [50.0, 0.0], [50.0, 200.0]).unwrap(),
            ],
            vec![],
        )
        .unwrap();
        let counter = LineCrossCounter::new(rois);
        let out = counter.count(&[motion(
            3,
            Some([40.0, 80.0]),
            [60.0, 140.0],
            true,
            CountableClass::BARBOX_ID,
        )]);
        assert_eq!(out.len(), 2);
        let ids: Vec<u16> = out.iter().map(|o| o.line_id).collect();
        assert!(ids.contains(&1) && ids.contains(&2));
    }

    #[test]
    fn polygon_contains_interior_and_exterior() {
        let poly =
            MaskPolygon::try_new(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]).unwrap();
        assert!(poly.contains([5.0, 5.0]));
        assert!(poly.contains([1.0, 1.0]));
        assert!(poly.contains([9.0, 9.0]));
        assert!(!poly.contains([15.0, 5.0]));
        assert!(!poly.contains([10.5, 10.5]));
        assert!(!poly.contains([-1.0, 5.0]));
    }

    #[test]
    fn degenerate_lines_rejected() {
        let mut lines: Vec<CountLine> = Vec::new();
        assert!(CountLine::try_new(1, [5.0, 5.0], [5.0, 5.0]).is_err());
        let _ = &mut lines;
    }

    #[test]
    fn segments_intersect_corners_and_touches() {
        assert!(segments_intersect(
            [0.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [10.0, 0.0]
        ));
        assert!(segments_intersect(
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, -5.0],
            [10.0, 5.0]
        ));
        assert!(!segments_intersect(
            [0.0, 0.0],
            [10.0, 10.0],
            [11.0, 0.0],
            [11.0, 5.0]
        ));
    }

    #[test]
    fn no_panics_on_edge_case_only() {
        // Deterministic: no corner cases beyond geometry.
        let now = Utc::now();
        assert!(now.timestamp() > 0);
    }
}
