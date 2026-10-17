use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: impl Into<String>) -> Result<Self, crate::error::CommonTypesError> {
                let v = value.into();
                if v.trim().is_empty() {
                    Err(crate::error::CommonTypesError::InvalidId(stringify!($name).to_string()))
                } else {
                    Ok(Self(v))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = crate::error::CommonTypesError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_new(s)
            }
        }
    };
}

string_id!(
    /// Stable factory-zone key, e.g. `"line_2"` (docs/11 `zones.zone_id`).
    ZoneId
);
string_id!(
    /// Logical camera handle inside the local RTSP VLAN.
    CameraId
);
string_id!(
    /// Operator login (SSO subject or magic-link identity).
    OperatorId
);
string_id!(
    /// Detector/fusion model identity, e.g. `"rfdetr-base-v3"` (`model_registry`).
    ModelVersion
);

impl ModelVersion {
    pub fn with_footnote(raw: &str, footnote: &str) -> Option<Self> {
        ModelVersion::try_new(format!("{raw}:{footnote}")).ok()
    }
}

/// ByteTrack identity of a package track across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(u64);

impl TrackId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Lazy-resolvable ring-buffer frame handle for an emitted event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRef {
    pub camera_id: CameraId,
    pub ring_seq: u64,
}

impl FrameRef {
    pub const fn new(camera_id: CameraId, ring_seq: u64) -> Self {
        Self {
            camera_id,
            ring_seq,
        }
    }
}
