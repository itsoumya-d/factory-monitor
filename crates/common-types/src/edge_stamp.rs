use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Alarm threshold from docs/13 §3: clock skew beyond this degrades fusion.
pub const MAX_PTP_OFFSET_NS: i64 = 25_000_000;

/// Edge-authoritative timestamp stamped at decode/receive time.
///
/// Wall clocks come from the local NTP/PTP-corrected clock — never from the
/// camera-side RTSP header. When the clock is degraded, downstream fusion must
/// widen alignment tolerance and force a `CLOCK_DEGRADED` uncertainty flag.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeStamp {
    pub utc: DateTime<Utc>,
    pub stream_pts: u64,
    pub ptp_offset_ns: i64,
    pub clock_ok: bool,
}

impl EdgeStamp {
    pub const fn new(
        utc: DateTime<Utc>,
        stream_pts: u64,
        ptp_offset_ns: i64,
        clock_ok: bool,
    ) -> Self {
        Self {
            utc,
            stream_pts,
            ptp_offset_ns,
            clock_ok,
        }
    }

    /// True when fusion must treat this timestamp as unreliable.
    pub fn is_degraded(&self) -> bool {
        !self.clock_ok || self.ptp_offset_ns.abs() > MAX_PTP_OFFSET_NS
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::TimeZone;

    fn stamp(offset_ns: i64, clock_ok: bool) -> EdgeStamp {
        let utc = Utc.with_ymd_and_hms(2026, 10, 17, 8, 0, 0).unwrap();
        EdgeStamp::new(utc, 1_000, offset_ns, clock_ok)
    }

    #[test]
    fn healthy_clock_is_not_degraded() {
        assert!(!stamp(0, true).is_degraded());
        assert!(!stamp(MAX_PTP_OFFSET_NS, true).is_degraded());
    }

    #[test]
    fn offset_beyond_threshold_degrades() {
        assert!(stamp(MAX_PTP_OFFSET_NS + 1, true).is_degraded());
        assert!(stamp(-(MAX_PTP_OFFSET_NS + 1), true).is_degraded());
    }

    #[test]
    fn lost_clock_always_degrades_even_with_zero_offset() {
        assert!(stamp(0, false).is_degraded());
    }

    #[test]
    fn serde_roundtrip_preserves_fields() {
        let s = stamp(1_234, true);
        let json = serde_json::to_string(&s).unwrap();
        let back: EdgeStamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
