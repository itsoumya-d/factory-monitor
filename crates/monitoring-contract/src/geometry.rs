//! Camera-agnostic physical geometry: zones, polygons, guard lines.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    /// Normalized image coordinate, [0.0, 1.0] inclusive.
    pub x: f32,
    pub y: f32,
}

impl GeoPoint {
    fn finite_and_bounded(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polygon(pub Vec<GeoPoint>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardLine {
    pub line_id: String,
    pub a: GeoPoint,
    pub b: GeoPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Ab,
    Ba,
    Any,
}

/// Dwell escalation in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DwellPolicy {
    pub warn_after_s: u32,
    pub alert_after_s: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractZone {
    pub zone: ZoneId,
    /// Camera channels observing this zone (may be several).
    pub cameras: Vec<String>,
    pub container: Polygon,
    pub guard_lines: Vec<GuardLine>,
    pub dwell_policy: Option<DwellPolicy>,
}

impl Polygon {
    pub fn validate(&self) -> Result<(), GeometryError> {
        if self.0.len() < 3 {
            return Err(GeometryError::TooFewVertices(self.0.len()));
        }
        if !self.0.iter().all(GeoPoint::finite_and_bounded) {
            return Err(GeometryError::VertexOutOfRange);
        }
        Ok(())
    }

    /// Absolute shoelace area in normalized units².
    pub fn area(&self) -> f32 {
        let mut twice = 0.0;
        for i in 0..self.0.len() {
            let a = self.0[i];
            let b = self.0[(i + 1) % self.0.len()];
            twice += a.x * b.y - b.x * a.y;
        }
        (twice / 2.0).abs()
    }
}

impl GuardLine {
    pub fn validate(&self) -> Result<(), GeometryError> {
        if !(self.a.finite_and_bounded() && self.b.finite_and_bounded()) {
            return Err(GeometryError::VertexOutOfRange);
        }
        // Degenerate segment (both endpoints identical) cannot define a crossing.
        if (self.a.x - self.b.x).abs() < f32::EPSILON && (self.a.y - self.b.y).abs() < f32::EPSILON
        {
            return Err(GeometryError::DegenerateLine(self.line_id.clone()));
        }
        Ok(())
    }
}

impl DwellPolicy {
    pub fn validate(&self) -> Result<(), GeometryError> {
        if self.alert_after_s <= self.warn_after_s {
            return Err(GeometryError::DwellPolicyInconsistent);
        }
        Ok(())
    }
}

impl ContractZone {
    pub fn validate(&self) -> Result<(), GeometryError> {
        if self.zone.0.is_empty() || self.zone.0.len() > 64 {
            return Err(GeometryError::ZoneIdMalformed(self.zone.0.clone()));
        }
        if self.cameras.is_empty() {
            return Err(GeometryError::ZoneWithoutCameras(self.zone.0.clone()));
        }
        self.container.validate()?;
        for line in &self.guard_lines {
            line.validate()?;
        }
        if let Some(dwell) = self.dwell_policy {
            dwell.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum GeometryError {
    #[error("polygon needs at least 3 vertices, got {0}")]
    TooFewVertices(usize),
    #[error("vertex outside [0,1] or non-finite")]
    VertexOutOfRange,
    #[error("degenerate guard line: {0}")]
    DegenerateLine(String),
    #[error("dwell alert threshold must exceed warn threshold")]
    DwellPolicyInconsistent,
    #[error("zone id malformed: {0}")]
    ZoneIdMalformed(String),
    #[error("zone {0} has no observing cameras")]
    ZoneWithoutCameras(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> GeoPoint {
        GeoPoint { x, y }
    }

    #[test]
    fn polygon_bounds_and_area() {
        let square = Polygon(vec![pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0), pt(0.0, 1.0)]);
        assert!(square.validate().is_ok());
        assert!((square.area() - 1.0).abs() < 1e-6);

        let bad = Polygon(vec![pt(0.0, 0.0), pt(1.0, 0.0)]);
        assert!(bad.validate().is_err());

        let outside = Polygon(vec![pt(0.0, 0.0), pt(1.0, 0.0), pt(1.5, 0.5)]);
        assert!(outside.validate().is_err());
    }

    #[test]
    fn guard_line_degeneracy() {
        let ok = GuardLine {
            line_id: "l1".into(),
            a: pt(0.0, 0.5),
            b: pt(1.0, 0.5),
        };
        assert!(ok.validate().is_ok());

        let degenerate = GuardLine {
            line_id: "l2".into(),
            a: pt(0.2, 0.2),
            b: pt(0.2, 0.2),
        };
        assert!(degenerate.validate().is_err());
    }

    #[test]
    fn dwell_policy_wiring() {
        let bad = DwellPolicy {
            warn_after_s: 30,
            alert_after_s: 30,
        };
        assert!(bad.validate().is_err());
        let good = DwellPolicy {
            warn_after_s: 30,
            alert_after_s: 60,
        };
        assert!(good.validate().is_ok());
    }
}
