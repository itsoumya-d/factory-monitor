//! EOD root-cause localization (docs/09 §3). Rust-side order of
//! operations: rank count events by uncertainty, cluster into 1-minute
//! culprit windows, sample ≤ 12 ambiguous frames per window, prompt the
//! VLM, then apply the answer to the zone failure profile.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

use common_types::counting::PackageCountEvent;

use crate::error::VlmError;
use crate::provider::VlmRequest;
use crate::schema;

pub const TASK_ROOT_CAUSE: &str = "root_cause_localization";
pub const FRAME_SAMPLE_CAP: usize = 12;
pub const DEFAULT_MAX_WINDOWS: usize = 3;

/// The projection of a `PackageCountEvent` the ranker sees.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub ts: DateTime<Utc>,
    pub zone_id: String,
    pub n_pred: i64,
    /// `uncertainty_score`: flag count + (1 − confidence), matching the
    /// docs intent of "sort by (uncertainty_score, divergence) desc".
    pub uncertainty_score: f32,
    /// |α_vision − α_wifi| at the tick (modality divergence).
    pub divergence: f32,
}

impl From<&PackageCountEvent> for EventRow {
    fn from(e: &PackageCountEvent) -> Self {
        let flag_count = e.uncertainty.iter().count() as f32;
        let blend = e.modality_blend.as_tuple();
        Self {
            ts: e.ts_from,
            zone_id: e.zone_id.to_string(),
            n_pred: e.count_delta,
            uncertainty_score: flag_count + (1.0 - e.confidence).clamp(0.0, 1.0),
            divergence: (blend.0 - blend.1).abs(),
        }
    }
}

/// One candidate episode: contiguous 1-minute buckets that contain events.
#[derive(Debug, Clone, PartialEq)]
pub struct MinuteWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub max_uncertainty: f32,
    pub total_divergence: f32,
}

/// Rank + cluster: events fall into minute buckets; adjacent non-empty
/// buckets merge into a window. Windows sort by (max_uncertainty,
/// total_divergence) desc (docs/09 §3 steps 1–2) and we keep the top
/// `max_windows`.
pub fn top_culprit_windows(rows: &[EventRow], max_windows: usize) -> Vec<MinuteWindow> {
    if rows.is_empty() || max_windows == 0 {
        return Vec::new();
    }
    let mut sorted: Vec<&EventRow> = rows.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.ts));

    // minute bucket key = epoch minutes
    let start_of_minute = |ts: DateTime<Utc>| {
        chrono::TimeZone::timestamp_opt(&Utc, ts.timestamp() - ts.timestamp().rem_euclid(60), 0)
            .earliest()
            .unwrap_or(ts)
    };
    let buckets: Vec<(i64, Vec<&EventRow>)> = {
        let mut map: std::collections::BTreeMap<i64, Vec<&EventRow>> =
            std::collections::BTreeMap::new();
        for r in sorted {
            let key = start_of_minute(r.ts).timestamp() / 60;
            map.entry(key).or_default().push(r);
        }
        map.into_iter().collect()
    };

    // Merge adjacent minute buckets into windows.
    let mut windows: Vec<MinuteWindow> = Vec::new();
    let mut it = buckets.into_iter().peekable();
    while let Some((key, rows)) = it.next() {
        let mut last_key = key;
        let mut max_u = 0.0f32;
        let mut total_div = 0.0f32;
        for r in &rows {
            max_u = max_u.max(r.uncertainty_score);
            total_div += r.divergence;
        }
        while let Some(&(nkey, _)) = it.peek() {
            if nkey != last_key + 1 {
                break;
            }
            let (_, next_rows) = match it.next() {
                Some(pair) => pair,
                None => break,
            };
            for r in next_rows {
                max_u = max_u.max(r.uncertainty_score);
                total_div += r.divergence;
            }
            last_key = nkey;
        }
        windows.push(MinuteWindow {
            start: start_of_minute(
                Utc.timestamp_opt(key * 60, 0)
                    .earliest()
                    .unwrap_or(Utc::now()),
            ),
            end: start_of_minute(
                Utc.timestamp_opt((last_key + 1) * 60, 0)
                    .earliest()
                    .unwrap_or(Utc::now()),
            ),
            max_uncertainty: max_u,
            total_divergence: total_div,
        });
    }

    windows.sort_by(|a, b| {
        b.max_uncertainty
            .partial_cmp(&a.max_uncertainty)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.total_divergence
                    .partial_cmp(&a.total_divergence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    windows.truncate(max_windows);
    windows
}

/// Evenly-thinned in-window frames (docs/09 §3 step 3: ≤ 12 per window).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSample {
    pub ts: DateTime<Utc>,
    pub frame_b64: String,
    pub pre_boxes: Vec<[f32; 4]>,
}

pub fn sample_frames(frames: &[FrameSample], window: &MinuteWindow) -> Vec<FrameSample> {
    let in_window: Vec<&FrameSample> = frames
        .iter()
        .filter(|f| f.ts >= window.start && f.ts < window.end)
        .collect();
    if in_window.len() <= FRAME_SAMPLE_CAP {
        return in_window.into_iter().cloned().collect();
    }
    let stride = in_window.len() as f64 / FRAME_SAMPLE_CAP as f64;
    let mut picked = Vec::with_capacity(FRAME_SAMPLE_CAP);
    for i in 0..FRAME_SAMPLE_CAP {
        let idx = (i as f64 * stride).floor() as usize;
        if let Some(f) = in_window.get(idx.min(in_window.len() - 1)) {
            picked.push((*f).clone());
        }
    }
    picked
}

#[derive(Debug, Clone, PartialEq)]
pub struct CulpritEpisode {
    pub window: MinuteWindow,
    pub frames: Vec<FrameSample>,
}

/// Build the batch prompt (docs/09 §3 step 3 schema).
pub fn build_root_cause_request(
    predicted: i64,
    actual: i64,
    known_tags: &[String],
    episodes: &[CulpritEpisode],
) -> Result<VlmRequest, VlmError> {
    if episodes.is_empty() {
        return Err(VlmError::InvalidPayload(
            "no culprit windows supplied".to_string(),
        ));
    }
    let windows: Vec<Value> = episodes
        .iter()
        .map(|ep| {
            json!({
                "ts": ep.window.start.to_rfc3339(),
                "ts_end": ep.window.end.to_rfc3339(),
                "frame_b64": ep.frames.first().map(|f| f.frame_b64.clone()).unwrap_or_default(),
                "pre_boxes": ep
                    .frames
                    .first()
                    .map(|f| f.pre_boxes.clone())
                    .unwrap_or_default(),
            })
        })
        .collect();
    Ok(VlmRequest::new(
        TASK_ROOT_CAUSE,
        json!({
            "task": TASK_ROOT_CAUSE,
            "counts": {"predicted": predicted, "actual": actual},
            "known_tags": known_tags,
            "windows": windows,
        }),
    ))
}

pub fn expected_schema() -> schema::JsonSchemaSpec {
    schema::root_cause_schema()
}

#[derive(Debug, Clone, PartialEq)]
pub struct CocoBox {
    pub file_name: String,
    pub bbox_2d: Vec<f64>,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RootCauseReport {
    pub culprit_start: DateTime<Utc>,
    pub culprit_end: DateTime<Utc>,
    pub root_cause: String,
    pub corrected_boxes: Vec<CocoBox>,
    pub notes: String,
}

/// Parse the answer from docs/09 §3 step 4. Timestamps come back as
/// RFC3339 strings; unparseable ones fail closed.
pub fn parse_root_cause_response(value: &Value) -> Result<RootCauseReport, VlmError> {
    let obj = value
        .as_object()
        .ok_or(VlmError::Schema("root-cause answer not an object".into()))?;
    let win = obj
        .get("culprit_window")
        .ok_or(VlmError::Schema("culprit_window missing".into()))?;
    let ts_start = win
        .get("ts_start")
        .and_then(Value::as_str)
        .ok_or(VlmError::Schema("culprit_window.ts_start missing".into()))?;
    let ts_end = win
        .get("ts_end")
        .and_then(Value::as_str)
        .ok_or(VlmError::Schema("culprit_window.ts_end missing".into()))?;
    let start = DateTime::parse_from_rfc3339(ts_start)
        .map_err(|e| VlmError::Schema(format!("culprit ts_start invalid: {e}")))?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(ts_end)
        .map_err(|e| VlmError::Schema(format!("culprit ts_end invalid: {e}")))?
        .with_timezone(&Utc);
    if end <= start {
        return Err(VlmError::Schema("culprit window empty/inverted".into()));
    }
    let mut boxes = Vec::new();
    for item in obj
        .get("corrected_boxes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        boxes.push(CocoBox {
            file_name: item
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            category: item
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            bbox_2d: item
                .get("bbox_2d")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default(),
        });
    }
    Ok(RootCauseReport {
        culprit_start: start,
        culprit_end: end,
        root_cause: obj
            .get("root_cause")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        corrected_boxes: boxes,
        notes: obj
            .get("notes")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

/// `zone_failure_profile` counters adjusted by this report (docs/09 §3
/// step 5a), consumed by the next morning pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FailureProfile {
    pub missed: i64,
    pub double_counted: i64,
    pub misclassified: i64,
}

impl FailureProfile {
    /// delta = reported(machine) − actual(human truth): positive means the
    /// engine over-counted (doubles), negative means it under-counted.
    pub fn apply(&mut self, reported: i64, actual: i64, report: &RootCauseReport) {
        let delta = reported - actual;
        if delta > 0 {
            self.double_counted += delta;
        } else if delta < 0 {
            self.missed += -delta;
        }
        self.misclassified += report.corrected_boxes.len() as i64;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::Duration;
    use common_types::bounds::DetectorThresholds;
    use common_types::counting::UncertaintyFlags;
    use common_types::fusion::ModalityBlend;
    use common_types::ids::{CameraId, FrameRef, ModelVersion, TrackId, ZoneId};

    fn at(min: u32, extra_secs: u64) -> DateTime<Utc> {
        Utc.timestamp_opt(0, 0).earliest().unwrap_or_else(Utc::now)
            + Duration::minutes(i64::from(min))
            + Duration::seconds(extra_secs as i64)
    }

    fn event(min: u32, conf: f32, blend_vision: f32, flags: UncertaintyFlags) -> EventRow {
        let e = PackageCountEvent::try_new(
            at(min, 0),
            at(min, 5),
            ZoneId::try_new("line_1").expect("zone id"),
            7,
            99,
            conf,
            flags,
            ModalityBlend::normalize(blend_vision),
            DetectorThresholds::try_new(
                common_types::bounds::CONF_DEFAULT,
                common_types::bounds::IOU_DEFAULT,
                common_types::bounds::TRACK_BUFFER_DEFAULT,
            )
            .unwrap_or_else(|e| panic!("{e}")),
            vec![TrackId::new(42)],
            vec![FrameRef::new(
                CameraId::try_new("cam_a").expect("camera id"),
                1,
            )],
            ModelVersion::try_new("p5-test").expect("model version"),
        )
        .expect("event");
        EventRow::from(&e)
    }

    #[test]
    fn event_row_projects_flags_confidence_and_divergence() {
        let row = event(0, 0.80, 0.70, UncertaintyFlags::OCCLUDED_ESTIMATE);
        // flags count 1 + (1 - 0.80)
        assert!(
            (row.uncertainty_score - 1.2).abs() < 1e-4,
            "{}",
            row.uncertainty_score
        );
        // |0.70 - 0.30|
        assert!((row.divergence - 0.4).abs() < 1e-4, "{}", row.divergence);
        assert_eq!(row.zone_id, "line_1");
        assert_eq!(row.n_pred, 7);
        let clean = event(1, 0.99, 0.50, UncertaintyFlags::empty());
        assert!((clean.uncertainty_score - 0.01).abs() < 1e-4);
        assert!(clean.divergence < 1e-4);
    }

    #[test]
    fn windows_rank_by_uncertainty_then_divergence() {
        let rows = vec![
            event(0, 0.9, 0.5, UncertaintyFlags::empty()), // 14:00
            event(1, 0.2, 0.9, UncertaintyFlags::MODALITY_DIVERGENCE), // 14:01 strong
            event(61, 0.3, 0.9, UncertaintyFlags::OCCLUDED_ESTIMATE), // 15:01
            event(62, 0.1, 0.4, UncertaintyFlags::CLOCK_DEGRADED), // 15:02
            event(122, 0.5, 0.5, UncertaintyFlags::LOW_TRACK_STRENGTH), // 16:02
        ];
        let top = top_culprit_windows(&rows, 3);
        assert_eq!(top.len(), 3);
        // 01:01–01:02 window max = flags 1 + 0.9 = 1.9 wins.
        assert_eq!(top[0].start.to_rfc3339(), "1970-01-01T01:01:00+00:00");
        assert_eq!(top[0].end.to_rfc3339(), "1970-01-01T01:03:00+00:00");
        // Minutes 00:00+00:01 are contiguous → ONE window, max 1.8.
        assert_eq!(top[1].start.to_rfc3339(), "1970-01-01T00:00:00+00:00");
        assert_eq!(top[1].end.to_rfc3339(), "1970-01-01T00:02:00+00:00");
        // 02:02 standalone window (1.5) third.
        assert_eq!(top[2].start.to_rfc3339(), "1970-01-01T02:02:00+00:00");
        assert_eq!(top[2].end.to_rfc3339(), "1970-01-01T02:03:00+00:00");
    }

    #[test]
    fn sample_frames_caps_at_twelve_evenly() {
        let window = MinuteWindow {
            start: at(0, 0),
            end: at(1, 0),
            max_uncertainty: 1.0,
            total_divergence: 0.0,
        };
        let frames: Vec<FrameSample> = (0..30)
            .map(|i| FrameSample {
                ts: at(0, i * 2), // 0-58s all inside the minute
                frame_b64: format!("{i}"),
                pre_boxes: vec![[0.0, 0.0, 1.0, 1.0]],
            })
            .collect();
        let picked = sample_frames(&frames, &window);
        assert_eq!(picked.len(), FRAME_SAMPLE_CAP);
        assert_eq!(picked[0].frame_b64, "0");
        // stride 30/12 = 2.5 → i=11 lands at floor(27.5) = 27
        assert_eq!(picked[11].frame_b64, "27");
        let small = sample_frames(&frames[..5], &window);
        assert_eq!(small.len(), 5, "small windows keep everything");
    }

    #[test]
    fn parse_root_cause_response_happy_and_bad_paths() {
        let value = serde_json::json!({
            "culprit_window": {"ts_start": "2026-04-07T14:01:00Z", "ts_end": "2026-04-07T14:03:00Z"},
            "root_cause": "pallets misclassified as boxes",
            "corrected_boxes": [
                {"file_name": "f0.jpg", "bbox_2d": [1.0, 2.0, 3.0, 4.0], "category": "pallet"}
            ],
            "notes": "operator confirmed"
        });
        let report = parse_root_cause_response(&value).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.root_cause, "pallets misclassified as boxes");
        assert_eq!(report.corrected_boxes.len(), 1);
        assert_eq!(
            report.culprit_start.to_rfc3339(),
            "2026-04-07T14:01:00+00:00"
        );
        assert_eq!(report.culprit_end.to_rfc3339(), "2026-04-07T14:03:00+00:00");

        let mut inverted = value.clone();
        inverted["culprit_window"]["ts_end"] = "2026-04-07T14:00:00Z".into();
        assert!(parse_root_cause_response(&inverted).is_err());

        let mut bad_ts = value;
        bad_ts["culprit_window"]["ts_start"] = "yesterday-ish".into();
        assert!(parse_root_cause_response(&bad_ts).is_err());
    }

    #[test]
    fn failure_profile_counts_doubles_misses_and_misclass() {
        let report = RootCauseReport {
            culprit_start: at(0, 0),
            culprit_end: at(2, 0),
            root_cause: "pallets".into(),
            corrected_boxes: vec![
                CocoBox {
                    file_name: "f.jpg".into(),
                    bbox_2d: vec![0.0; 4],
                    category: "pallet".into(),
                },
                CocoBox {
                    file_name: "g.jpg".into(),
                    bbox_2d: vec![0.0; 4],
                    category: "package".into(),
                },
            ],
            notes: String::new(),
        };
        let mut profile = FailureProfile::default();
        profile.apply(1485, 1420, &report); // overcount by 65
        assert_eq!(profile.double_counted, 65);
        assert_eq!(profile.missed, 0);
        assert_eq!(profile.misclassified, 2);
        profile.apply(1400, 1420, &report); // undercount by 20
        assert_eq!(profile.double_counted, 65);
        assert_eq!(profile.missed, 20);
        assert_eq!(profile.misclassified, 4);
    }

    #[test]
    fn batch_request_carries_counts_tags_and_windows() {
        let window = MinuteWindow {
            start: at(0, 0),
            end: at(1, 0),
            max_uncertainty: 1.0,
            total_divergence: 0.5,
        };
        let ep = CulpritEpisode {
            window,
            frames: vec![FrameSample {
                ts: at(0, 10),
                frame_b64: "AA".into(),
                pre_boxes: vec![[0.0, 0.0, 2.0, 2.0]],
            }],
        };
        let req = build_root_cause_request(1485, 1420, &["PALLET_AS_BOX".into()], &[ep])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(req.task, TASK_ROOT_CAUSE);
        let counts = req
            .body
            .get("counts")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("counts"));
        assert_eq!(counts.get("predicted"), Some(&json!(1485)));
        assert_eq!(counts.get("actual"), Some(&json!(1420)));
        let windows = req
            .body
            .get("windows")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("windows"));
        assert_eq!(windows.len(), 1);
        assert!(build_root_cause_request(1, 1, &[], &[]).is_err());
    }
}
