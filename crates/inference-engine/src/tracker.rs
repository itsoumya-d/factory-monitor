//! ByteTrack (docs/07 §3): two-stage association with Hungarian assignment,
//! an 8-state Kalman filter per track, pending confirmation, score EMA
//! smoothing, zombie (deauthorized) rescue, and the RL-adjustable track
//! buffer.
//!
//! Determinism notes: detections are stably sorted by score (DESC); each
//! association stage runs `lapjv` on a square cost matrix (1 − IoU, ∞ for
//! gate-failures) which returns the unique optimal assignment for our input
//! unless gates tie — and then the first (highest-score) detection wins by
//! construction of the stable sort.

use std::collections::VecDeque;

use lapjv::{Matrix, lapjv};
use nalgebra::{SMatrix, SVector};

use common_types::TrackId;
use common_types::traits::Detection;

use crate::error::InferenceError;

/// Association gates and lifecycle knobs. Stage gates follow docs/07 §3 with
/// `τ_iou` (the RL association gate) driving stage 1; stage 2 and rescue sit
/// 0.15 / 0.10 below it, which reproduces the docs' 0.50/0.35/0.40 triple at
/// `τ_iou = 0.50`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackParams {
    pub iou_first: f32,
    pub iou_second: f32,
    pub iou_rescue: f32,
    /// A track becomes `Confirmed` after this many consecutive associations.
    pub confirm_frames: u32,
    /// Zombie grace in frames — the RL `track_buffer_frames`.
    pub buffer_frames: u32,
    /// Score EMA weight of the new detection score (docs/07 §3 α = 0.6).
    pub score_alpha: f32,
}

impl TrackParams {
    pub fn from_thresholds(iou_threshold: f32, track_buffer_frames: u16) -> Self {
        let iou = iou_threshold.clamp(0.25, 0.70);
        Self {
            iou_first: iou,
            iou_second: (iou - 0.15).max(0.25),
            iou_rescue: (iou - 0.10).max(0.30),
            confirm_frames: 2,
            buffer_frames: u32::from(track_buffer_frames),
            score_alpha: 0.6,
        }
    }

    /// Docs' reference triple (0.50, 0.35, 0.40) with a 45-frame buffer.
    pub fn docs_default() -> Self {
        Self::from_thresholds(0.50, 45)
    }
}

/// Visibility of a track inside the tracker (docs/07 §3 states).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackState {
    /// Waiting for `confirm_frames` associations.
    Pending,
    /// Countable.
    Confirmed,
    /// Deauthorized: buffered `buffer_frames` longer for low-score rescue.
    Zombie,
}

/// Tracker output for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedBox {
    pub track_id: TrackId,
    /// KF-posterior box in frame pixel coords, `[x1, y1, x2, y2]`.
    pub bbox_xyxy: [f32; 4],
    /// EMA-smoothed score.
    pub score: f32,
    pub class_id: u16,
    pub state: TrackState,
    /// Centroid from the previous frame, when the track has ≥ 2-frame
    /// history (crossing consumers, docs/07 §4).
    pub prev_centroid: Option<[f32; 2]>,
    /// Current-frame centroid.
    pub centroid: [f32; 2],
}

impl TrackedBox {
    pub fn confirmed(&self) -> bool {
        self.state == TrackState::Confirmed
    }
}

#[derive(Debug, Clone, Copy)]
struct CentroidStamp {
    frame: u64,
    cx: f32,
    cy: f32,
}

/// `[cx, cy, w, h, vx, vy, vw, vh]` (docs/07 §3), `nalgebra` `SMatrix`.
#[derive(Debug, Clone)]
struct KfState {
    x: SVector<f32, 8>,
    c: SMatrix<f32, 8, 8>,
}

impl KfState {
    fn init(cx: f32, cy: f32, w: f32, h: f32) -> Self {
        let short_side = w.min(h).max(1.0);
        // ByteTrack-style generous init covariance.
        let mut c = SMatrix::<f32, 8, 8>::identity();
        c[(0, 0)] = (2.0 * short_side) * (2.0 * short_side);
        c[(1, 1)] = c[(0, 0)];
        c[(2, 2)] = 100.0;
        c[(3, 3)] = 100.0;
        c[(4, 4)] = 625.0;
        c[(5, 5)] = 625.0;
        c[(6, 6)] = 900.0;
        c[(7, 7)] = 900.0;
        Self {
            x: SVector::<f32, 8>::from_column_slice(&[cx, cy, w, h, 0.0, 0.0, 0.0, 0.0]),
            c,
        }
    }

    fn predict(&mut self) {
        // F = identity + velocity integration (dt = 1 frame).
        let mut f = SMatrix::<f32, 8, 8>::identity();
        f[(0, 4)] = 1.0;
        f[(1, 5)] = 1.0;
        f[(2, 6)] = 1.0;
        f[(3, 7)] = 1.0;
        self.x = f * self.x;
        let q = SMatrix::<f32, 8, 8>::from_diagonal(&SVector::<f32, 8>::from_column_slice(&[
            0.25, 0.25, 0.25, 0.25, 10.0, 10.0, 2.5, 2.5,
        ]));
        self.c = f * self.c * f.transpose() + q;
    }

    fn update(&mut self, cx: f32, cy: f32, w: f32, h: f32) {
        let r = SMatrix::<f32, 4, 4>::from_diagonal(&SVector::<f32, 4>::from_column_slice(&[
            2.0, 2.0, 2.0, 2.0,
        ]));
        let innovation = SVector::<f32, 4>::from_column_slice(&[cx, cy, w, h])
            - self.x.fixed_view::<4, 1>(0, 0).clone_owned();
        let s = self.c.fixed_view::<4, 4>(0, 0).clone_owned() + r;
        let s_inv = s
            .try_inverse()
            .unwrap_or_else(SMatrix::<f32, 4, 4>::identity);
        // K (8×4) = P·Hᵀ·S⁻¹; Hᵀ spans the first four state rows.
        let p_ht = self.c.fixed_view::<8, 4>(0, 0).clone_owned();
        let k = p_ht * s_inv;
        self.x += k * innovation;
        // P = (I − K·H) P.
        let mut i_kh = SMatrix::<f32, 8, 8>::identity();
        for i in 0..8 {
            for j in 0..4 {
                i_kh[(i, j)] -= k[(i, j)];
            }
        }
        self.c = i_kh * self.c;
    }

    fn bbox(&self) -> [f32; 4] {
        let cx = self.x[0];
        let cy = self.x[1];
        let w = self.x[2].max(1.0);
        let h = self.x[3].max(1.0);
        [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
    }

    fn centroid(&self) -> [f32; 2] {
        [self.x[0], self.x[1]]
    }
}

struct Track {
    id: TrackId,
    class_id: u16,
    score_ema: f32,
    kf: KfState,
    /// Stage-1 association anchor: the box matched in the last update
    /// (docs/07 §3 "IoU(track_last, det)").
    last_box: [f32; 4],
    state: TrackState,
    since_hit: u32,
    hits: u32,
    history: VecDeque<CentroidStamp>,
}

pub fn iou_xyxy(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = a[2].min(b[2]);
    let y2 = a[3].min(b[3]);
    let iw = (x2 - x1).max(0.0);
    let ih = (y2 - y1).max(0.0);
    let inter = iw * ih;
    let area_a = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let area_b = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let denom = area_a + area_b - inter;
    if denom <= 0.0 {
        0.0
    } else {
        (inter / denom).clamp(0.0, 1.0)
    }
}

/// Row → column optimal assignment over a square cost matrix. Rows/columns
/// whose index exceeds the real count are padding (∞). Returns `Some(col)`
/// for each real row that got a real, finite column.
fn assign_square(
    square: &Matrix<f64>,
    n_rows: usize,
    n_cols: usize,
) -> Result<Vec<Option<usize>>, InferenceError> {
    let dim = square.dim().0;
    if dim == 0 {
        return Ok(Vec::new());
    }
    let has_finite = square.iter().any(|v| v.is_finite());
    if !has_finite {
        return Ok(vec![None; dim]);
    }
    let (x, _y) = lapjv(square).map_err(|e| InferenceError::Assignment(e.to_string()))?;
    let mut out = vec![None; dim];
    for (row, col) in x.iter().enumerate() {
        if row < n_rows && *col < n_cols && square[(row, *col)].is_finite() {
            out[row] = Some(*col);
        }
    }
    Ok(out)
}

/// Square cost matrix of the gated IoU costs; ∞ cells exclude a pair.
fn decay_cost_matrix(row_boxes: &[[f32; 4]], col_boxes: &[[f32; 4]], gate: f32) -> Matrix<f64> {
    let n = row_boxes.len().max(col_boxes.len());
    let mut m: Matrix<f64> = Matrix::from_elem((n, n), f64::INFINITY);
    for (r, rb) in row_boxes.iter().enumerate() {
        for (c, cb) in col_boxes.iter().enumerate() {
            let iou = iou_xyxy(rb, cb);
            if iou >= gate {
                m[(r, c)] = (1.0 - iou) as f64;
            }
        }
    }
    m
}

/// ByteTrack per camera; owns the two-stage association and zombie lifecycle.
pub struct ByteTracker {
    tracks: Vec<Track>,
    next_id: u64,
    params: TrackParams,
    frame_seq: u64,
}

impl ByteTracker {
    pub fn new(params: TrackParams) -> Self {
        Self {
            tracks: Vec::new(),
            next_id: 1,
            params,
            frame_seq: 0,
        }
    }

    pub fn reconfigure(&mut self, params: TrackParams) {
        self.params = params;
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_seq
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn zombies(&self) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.state == TrackState::Zombie)
            .count()
    }

    pub fn confirmed_ids(&self) -> Vec<TrackId> {
        self.tracks
            .iter()
            .filter(|t| t.state == TrackState::Confirmed)
            .map(|t| t.id)
            .collect()
    }

    fn spawn(&mut self, det: &Detection) {
        let cx = (det.bbox_xyxy[0] + det.bbox_xyxy[2]) / 2.0;
        let cy = (det.bbox_xyxy[1] + det.bbox_xyxy[3]) / 2.0;
        let w = (det.bbox_xyxy[2] - det.bbox_xyxy[0]).max(1.0);
        let h = (det.bbox_xyxy[3] - det.bbox_xyxy[1]).max(1.0);
        let id = TrackId::new(self.next_id);
        self.next_id += 1;
        self.tracks.push(Track {
            id,
            class_id: det.class_id,
            score_ema: det.score,
            kf: KfState::init(cx, cy, w, h),
            last_box: det.bbox_xyxy,
            state: TrackState::Pending,
            since_hit: 0,
            hits: 1,
            history: VecDeque::from([CentroidStamp {
                frame: self.frame_seq,
                cx,
                cy,
            }]),
        });
    }

    fn prev_centroid(track: &Track, frame: u64) -> Option<[f32; 2]> {
        track
            .history
            .iter()
            .rev()
            .find(|s| s.frame < frame)
            .map(|s| [s.cx, s.cy])
    }

    fn apply_match(track: &mut Track, det: &Detection, frame: u64, alpha: f32) {
        let cx = (det.bbox_xyxy[0] + det.bbox_xyxy[2]) / 2.0;
        let cy = (det.bbox_xyxy[1] + det.bbox_xyxy[3]) / 2.0;
        let w = (det.bbox_xyxy[2] - det.bbox_xyxy[0]).max(1.0);
        let h = (det.bbox_xyxy[3] - det.bbox_xyxy[1]).max(1.0);
        track.kf.update(cx, cy, w, h);
        track.last_box = det.bbox_xyxy;
        track.score_ema = alpha * det.score + (1.0 - alpha) * track.score_ema;
        track.class_id = det.class_id;
        track.hits += 1;
        track.since_hit = 0;
        if track.state == TrackState::Pending && track.hits >= 2 {
            track.state = TrackState::Confirmed;
        } else if track.state == TrackState::Zombie {
            // Rescue has re-attached: deauthorize the deauthorization.
            track.state = TrackState::Confirmed;
        }
        track.history.push_back(CentroidStamp { frame, cx, cy });
        while track.history.len() > 8 {
            track.history.pop_front();
        }
    }

    /// One detector frame in. `detections` order is irrelevant; the result is
    /// deterministic for a given input.
    pub fn update(&mut self, detections: &[Detection]) -> Result<Vec<TrackedBox>, InferenceError> {
        self.frame_seq += 1;
        let frame = self.frame_seq;

        let mut dets: Vec<Detection> = detections.to_vec();
        dets.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for track in &mut self.tracks {
            track.kf.predict();
        }

        let mut matched_rows: Vec<Option<usize>> = vec![None; self.tracks.len()];
        let mut matched_dets = vec![false; dets.len()];

        // ---- Stage 1: IoU(track_last, det) ≥ gate over all active tracks.
        {
            let active_rows: Vec<usize> = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.state != TrackState::Zombie)
                .map(|(i, _)| i)
                .collect();
            let row_boxes: Vec<[f32; 4]> = active_rows
                .iter()
                .map(|&i| self.tracks[i].last_box)
                .collect();
            let col_boxes: Vec<[f32; 4]> = dets.iter().map(|d| d.bbox_xyxy).collect();
            let cost = decay_cost_matrix(&row_boxes, &col_boxes, self.params.iou_first);
            let assignment = assign_square(&cost, row_boxes.len(), col_boxes.len())?;
            for (ri, col) in assignment.iter().enumerate() {
                if let Some(col) = col {
                    matched_rows[active_rows[ri]] = Some(*col);
                    matched_dets[*col] = true;
                }
            }
        }

        // ---- Stage 2: re-confirmed tracks against unmatched dets via the
        // KF-predicted box.
        {
            let unmatched: Vec<usize> = (0..dets.len()).filter(|&i| !matched_dets[i]).collect();
            let rows: Vec<usize> = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.state == TrackState::Confirmed)
                .map(|(i, _)| i)
                .collect();
            let row_boxes: Vec<[f32; 4]> = rows.iter().map(|&i| self.tracks[i].kf.bbox()).collect();
            let col_boxes: Vec<[f32; 4]> = unmatched.iter().map(|&i| dets[i].bbox_xyxy).collect();
            let cost = decay_cost_matrix(&row_boxes, &col_boxes, self.params.iou_second);
            let assignment = assign_square(&cost, row_boxes.len(), col_boxes.len())?;
            for (ri, col) in assignment.iter().enumerate() {
                if let Some(col) = col {
                    let ti = rows[ri];
                    let det_idx = unmatched[*col];
                    if matched_rows[ti].is_none() && !matched_dets[det_idx] {
                        matched_rows[ti] = Some(det_idx);
                        matched_dets[det_idx] = true;
                    }
                }
            }
        }

        // ---- Apply matches.
        for (ti, track) in self.tracks.iter_mut().enumerate() {
            if let Some(c) = matched_rows[ti] {
                Self::apply_match(track, &dets[c], frame, self.params.score_alpha);
            } else {
                track.since_hit += 1;
                if track.since_hit > self.params.buffer_frames {
                    track.state = TrackState::Zombie;
                }
            }
        }

        // ---- Stage 3: rescue unmatched dets against deauthorized zombies.
        {
            let unmatched: Vec<usize> = (0..dets.len()).filter(|&i| !matched_dets[i]).collect();
            let rows: Vec<usize> = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.state == TrackState::Zombie)
                .map(|(i, _)| i)
                .collect();
            let row_boxes: Vec<[f32; 4]> = rows.iter().map(|&i| self.tracks[i].kf.bbox()).collect();
            let col_boxes: Vec<[f32; 4]> = unmatched.iter().map(|&i| dets[i].bbox_xyxy).collect();
            let cost = decay_cost_matrix(&row_boxes, &col_boxes, self.params.iou_rescue);
            let assignment = assign_square(&cost, row_boxes.len(), col_boxes.len())?;
            for (ri, col) in assignment.iter().enumerate() {
                if let Some(col) = col {
                    let ti = rows[ri];
                    let det_idx = unmatched[*col];
                    if !matched_dets[det_idx] {
                        matched_dets[det_idx] = true;
                        matched_rows[ti] = Some(det_idx);
                        Self::apply_match(
                            &mut self.tracks[ti],
                            &dets[det_idx],
                            frame,
                            self.params.score_alpha,
                        );
                    }
                }
            }
        }

        // ---- New pending tracks from the leftovers.
        for (c, matched) in matched_dets.iter().enumerate() {
            if !matched {
                self.spawn(&dets[c]);
            }
        }

        // ---- Zombie-kill after the rescue window (double buffer overall:
        // buffer miss grace + equal zombie grace, docs/07 §3).
        self.tracks.retain(|t| {
            !(t.state == TrackState::Zombie && t.since_hit > 2 * self.params.buffer_frames)
        });

        // ---- Output every live track.
        let mut out = Vec::with_capacity(self.tracks.len());
        for track in &self.tracks {
            out.push(TrackedBox {
                track_id: track.id,
                bbox_xyxy: track.kf.bbox(),
                score: track.score_ema,
                class_id: track.class_id,
                state: track.state,
                prev_centroid: Self::prev_centroid(track, frame),
                centroid: track.kf.centroid(),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    fn det_bbox(x1: f32, y1: f32, x2: f32, y2: f32, score: f32, class: u16) -> Detection {
        Detection {
            bbox_xyxy: [x1, y1, x2, y2],
            score,
            class_id: class,
        }
    }

    fn moving_dets(frame: u64) -> Vec<Detection> {
        vec![det_bbox(
            10.0 + 10.0 * frame as f32,
            100.0,
            110.0 + 10.0 * frame as f32,
            200.0,
            0.9,
            0,
        )]
    }

    fn center(b: [f32; 4]) -> [f32; 2] {
        [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]
    }

    #[test]
    fn pending_then_confirmed_after_two_associations() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        let out1 = tr.update(&moving_dets(0)).unwrap();
        assert_eq!(out1[0].state, TrackState::Pending);
        let out2 = tr.update(&moving_dets(1)).unwrap();
        assert_eq!(out2[0].state, TrackState::Confirmed);
        assert_eq!(out2[0].track_id, TrackId::new(1));
        assert_eq!(tr.confirmed_ids(), vec![TrackId::new(1)]);
    }

    #[test]
    fn constant_velocity_track_follows_and_learns() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        let mut last = None;
        for f in 0..10u64 {
            let out = tr.update(&moving_dets(f)).unwrap();
            assert_eq!(out.len(), 1, "one persistent track");
            assert_eq!(out[0].track_id, TrackId::new(1));
            last = Some(out.clone());
        }
        let final_out = last.unwrap();
        let target = center(moving_dets(9)[0].bbox_xyxy);
        let got = final_out[0].centroid;
        let drift = ((got[0] - target[0]).powi(2) + (got[1] - target[1]).powi(2)).sqrt();
        assert!(drift < 15.0, "centroid drifted {drift} px");
        let prev = final_out[0].prev_centroid.unwrap();
        assert!(
            (got[0] - prev[0]).abs() > 5.0,
            "frame-to-frame motion visible: {prev:?} -> {got:?}"
        );
    }

    #[test]
    fn score_ema_smooths_consecutive_scores() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        let out1 = tr
            .update(&[det_bbox(0.0, 0.0, 100.0, 100.0, 0.9, 0)])
            .unwrap();
        assert!((out1[0].score - 0.9).abs() < 1e-6);
        let out2 = tr
            .update(&[det_bbox(0.0, 0.0, 100.0, 100.0, 0.5, 0)])
            .unwrap();
        // α = 0.6: 0.6·0.5 + 0.4·0.9.
        assert!((out2[0].score - 0.66).abs() < 1e-5, "{}", out2[0].score);
    }

    #[test]
    fn unmatched_track_expires_into_zombie_then_rescues() {
        // Buffer 2 → zombie after 3 misses.
        let mut tr = ByteTracker::new(TrackParams::from_thresholds(0.50, 2));
        tr.update(&[det_bbox(0.0, 0.0, 100.0, 100.0, 0.9, 0)])
            .unwrap();
        for _ in 0..3 {
            tr.update(&[]).unwrap();
        }
        assert_eq!(tr.zombies(), 1);
        // Same object reappears overlapping: zombie rescue keeps the id.
        let out = tr
            .update(&[det_bbox(10.0, 10.0, 100.0, 100.0, 0.2, 0)])
            .unwrap()
            .into_iter()
            .find(|t| t.track_id == TrackId::new(1))
            .expect("rescued zombie keeps id 1");
        assert_eq!(out.state, TrackState::Confirmed);
    }

    #[test]
    fn zombie_killed_after_double_buffer() {
        let mut tr = ByteTracker::new(TrackParams::from_thresholds(0.50, 2));
        tr.update(&[det_bbox(0.0, 0.0, 100.0, 100.0, 0.9, 0)])
            .unwrap();
        for _ in 0..5 {
            tr.update(&[]).unwrap();
        }
        // Misses 1..2: active grace; miss 3: zombie (deauthorized); misses
        // 4..5: rescue window; at miss 5 the zombie must be gone.
        assert!(tr.is_empty(), "expected full zombie-kill");
    }

    #[test]
    fn two_objects_keep_two_ids() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        for f in 0..4u64 {
            let out = tr
                .update(&[
                    det_bbox(0.0 + 5.0 * f as f32, 0.0, 100.0, 100.0, 0.9, 0),
                    det_bbox(500.0, 0.0 + 3.0 * f as f32, 600.0, 100.0, 0.8, 0),
                ])
                .unwrap();
            let ids: std::collections::BTreeSet<u64> =
                out.iter().map(|t| t.track_id.as_u64()).collect();
            assert_eq!(ids.len(), 2);
            // Deterministic assignment: id 1 = left (higher score), id 2 = right.
            let left = out.iter().find(|t| t.track_id == TrackId::new(1)).unwrap();
            assert!(left.centroid[0] < 200.0);
        }
        assert_eq!(tr.confirmed_ids().len(), 2);
    }

    #[test]
    fn fast_object_falls_out_and_mints_a_new_id() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        tr.update(&[det_bbox(0.0, 0.0, 50.0, 50.0, 0.9, 0)])
            .unwrap();
        // Teleport far away: IoU below stage gates → new pending track.
        let out = tr
            .update(&[det_bbox(900.0, 0.0, 950.0, 50.0, 0.9, 0)])
            .unwrap();
        assert_eq!(out.len(), 2);
        let new_ids: Vec<u64> = out.iter().map(|t| t.track_id.as_u64()).collect();
        assert!(new_ids.contains(&2));
    }

    #[test]
    fn duplicate_update_calls_are_deterministic() {
        let run = || -> Vec<TrackedBox> {
            let mut tr = ByteTracker::new(TrackParams::docs_default());
            let _ = tr
                .update(&[
                    det_bbox(0.0, 0.0, 100.0, 100.0, 0.9, 0),
                    det_bbox(400.0, 0.0, 500.0, 100.0, 0.85, 1),
                ])
                .unwrap();
            tr.update(&[
                det_bbox(5.0, 0.0, 105.0, 100.0, 0.88, 0),
                det_bbox(405.0, 0.0, 505.0, 100.0, 0.83, 1),
            ])
            .unwrap()
        };
        let one = run();
        let two = run();
        assert_eq!(one, two);
    }

    #[test]
    fn empty_stream_keeps_running() {
        let mut tr = ByteTracker::new(TrackParams::docs_default());
        let out = tr.update(&[]).unwrap();
        assert!(out.is_empty());
        assert_eq!(tr.frame_count(), 1);
    }

    proptest! {
        #[test]
        fn iou_self_consistent(ax1 in 0.0f32..=1000.0, ay1 in 0.0f32..=1000.0,
            w in 1.0f32..=300.0, h in 1.0f32..=300.0,
            dx in -500.0f32..=500.0, dy in -500.0f32..=500.0)
        {
            let a = [ax1, ay1, ax1 + w, ay1 + h];
            let b = [ax1 + dx, ay1 + dy, ax1 + dx + w, ay1 + dy + h];
            let v = iou_xyxy(&a, &b);
            prop_assert!((0.0..=1.0).contains(&v));
            if dx.abs() >= w || dy.abs() >= h {
                prop_assert!(v.abs() < 1e-6, "disjoint boxes must have IoU 0, got {v}");
            }
            prop_assert!((iou_xyxy(&a, &a) - 1.0).abs() < 1e-6);
        }

        #[test]
        fn tracker_never_panics_or_duplicates_ids(
            seed_bits in proptest::collection::vec(0u32..1_000, 20)
        ) {
            let mut tr = ByteTracker::new(TrackParams::docs_default());
            for chunk in seed_bits.chunks(4) {
                let x1 = (chunk[0] % 800) as f32;
                let y1 = (chunk[1] % 600) as f32;
                let dets: Vec<Detection> = (0..chunk[2] % 5)
                    .map(|i| {
                        det_bbox(
                            x1 + (i * 90) as f32,
                            y1,
                            x1 + (i * 90) as f32 + 60.0,
                            y1 + 60.0,
                            0.3 + (chunk[3] % 70) as f32 / 100.0,
                            (i % 3) as u16,
                        )
                    })
                    .collect();
                let out = tr.update(&dets)?;
                // Ids must be unique within one frame (persistence across
                // frames is the point of tracking).
                let ids: std::collections::BTreeSet<u64> =
                    out.iter().map(|t| t.track_id.as_u64()).collect();
                prop_assert_eq!(ids.len(), out.len());
            }
        }
    }
}
