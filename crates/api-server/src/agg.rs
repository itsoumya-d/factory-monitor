//! Count aggregation (docs/10 §1 counts/hourly routes; docs/11 §conventions:
//! dashboards read aggregates, audits read raw events).

use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use common_types::counting::PackageCountEvent;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AggregatedCounts {
    pub zone_id: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total: i64,
    pub events: usize,
    pub avg_confidence: f32,
    pub occluded_events: usize,
}

/// Zip raw events into the aggregate the dashboard renders.
pub fn aggregate(
    zone_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    events: &[PackageCountEvent],
) -> AggregatedCounts {
    let total: i64 = events.iter().map(|e| e.count_delta).sum();
    let n = events.len().max(1);
    let avg = events.iter().map(|e| e.confidence).sum::<f32>() / n as f32;
    let occluded = events.iter().filter(|e| e.is_occluded_estimate()).count();
    AggregatedCounts {
        zone_id: zone_id.to_string(),
        from,
        to,
        total,
        events: events.len(),
        avg_confidence: avg,
        occluded_events: occluded,
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HourlyPoint {
    pub hour: DateTime<Utc>,
    pub packages: i64,
    pub avg_conf: f32,
    pub occluded_events: i64,
}

/// The hourly curve for the report view: hour buckets in UTC of `date`.
/// Events whose `ts_from` falls on another calendar day are excluded.
pub fn hourly_curve(date_ymd: (i32, u32, u32), events: &[PackageCountEvent]) -> Vec<HourlyPoint> {
    let day = Utc
        .with_ymd_and_hms(date_ymd.0, date_ymd.1, date_ymd.2, 0, 0, 0)
        .latest()
        .unwrap_or_else(Utc::now);
    let day_end = day + Duration::hours(24);
    let mut buckets: Vec<Option<Vec<&PackageCountEvent>>> =
        std::iter::repeat_with(|| None).take(24).collect();
    for e in events {
        if e.ts_from < day || e.ts_from >= day_end {
            continue;
        }
        let hour = e.ts_from.hour() as usize;
        buckets[hour].get_or_insert_with(Vec::new).push(e);
    }
    buckets
        .into_iter()
        .enumerate()
        .filter_map(|(h, bucket)| {
            let list = bucket?;
            let events_in_hour = list.len();
            let packages: i64 = list.iter().map(|e| e.count_delta).sum();
            let avg_conf = events_in_hour > 0;
            let conf = events_in_hour as u32;
            let occluded = list
                .iter()
                .filter(|e| e.is_occluded_estimate())
                .cloned()
                .count() as i64;
            Some(HourlyPoint {
                hour: day + Duration::hours(h as i64),
                packages,
                avg_conf: if avg_conf {
                    list.iter().map(|e| e.confidence).sum::<f32>() / conf as f32
                } else {
                    0.0
                },
                occluded_events: occluded,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_types::bounds::DetectorThresholds;
    use common_types::counting::UncertaintyFlags;
    use common_types::fusion::ModalityBlend;
    use common_types::ids::{ModelVersion, ZoneId};

    fn event(min_after_14: u32, delta: i64, conf: f32, occluded: bool) -> PackageCountEvent {
        let base = Utc
            .with_ymd_and_hms(2026, 4, 7, 14, 0, 0)
            .latest()
            .unwrap_or_else(Utc::now);
        let start = base + Duration::minutes(i64::from(min_after_14));
        PackageCountEvent::try_new(
            start,
            start + Duration::minutes(5),
            ZoneId::try_new("line_1").expect("zone"),
            delta,
            delta,
            conf,
            if occluded {
                UncertaintyFlags::OCCLUDED_ESTIMATE
            } else {
                UncertaintyFlags::empty()
            },
            ModalityBlend::normalize(0.7),
            DetectorThresholds::try_new(0.45, 0.45, 45).expect("th"),
            vec![],
            vec![],
            ModelVersion::try_new("t").expect("mv"),
        )
        .expect("event")
    }

    #[test]
    fn aggregate_sums_counts_and_occurrences() {
        let to = Utc.with_ymd_and_hms(2026, 4, 7, 15, 0, 0).unwrap();
        let from = Utc.with_ymd_and_hms(2026, 4, 7, 14, 0, 0).unwrap();
        let events = vec![event(1, 10, 0.9, false), event(2, 5, 0.7, true)];
        let agg = aggregate("line_1", from, to, &events);
        assert_eq!(agg.total, 15);
        assert_eq!(agg.events, 2);
        assert!((agg.avg_confidence - 0.8).abs() < 1e-5);
        assert_eq!(agg.occluded_events, 1);
        // Empty window is well-formed zeros (not a divide-by-zero NaN).
        let empty = aggregate("line_1", from, to, &[]);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.events, 0);
        assert_eq!(empty.avg_confidence, 0.0);
    }

    #[test]
    fn hourly_curve_buckets_within_one_day() {
        let events = vec![
            event(0, 10, 0.9, false),
            event(30, 4, 0.8, false),
            event(90, 3, 0.6, true), // 15:30, same clock hour bucket h=15
        ];
        let curve = hourly_curve((2026, 4, 7), &events);
        assert_eq!(curve.len(), 2, "14:00 and 15:00 buckets only");
        assert_eq!(curve[0].packages, 14);
        assert_eq!(curve[0].hour.to_rfc3339(), "2026-04-07T14:00:00+00:00");
        assert_eq!(curve[1].packages, 3);
        assert_eq!(curve[1].occluded_events, 1);
        // Cross-day events excluded entirely.
        let other = vec![
            {
                let mut e = event(0, 10, 0.9, false);
                e.ts_from = Utc.with_ymd_and_hms(2026, 4, 6, 14, 0, 0).unwrap();
                e.ts_to = Utc.with_ymd_and_hms(2026, 4, 6, 14, 5, 0).unwrap();
                e
            },
            event(10, 7, 0.9, false),
        ];
        let curve2 = hourly_curve((2026, 4, 7), &other);
        assert_eq!(curve2.len(), 1);
        assert_eq!(curve2[0].packages, 7);
    }
}
