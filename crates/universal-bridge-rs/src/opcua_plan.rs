//! OPC UA tag plan: cert paths + station tag namespace — the 'plan' is
//! CI-checked so a tag typo never reaches the production line.

use thiserror::Error;

pub const STATION_TAG_SLUG: &str = "ns=5;s=shado.monitor.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpcUaPlanError {
    #[error("tag {0:?} outside the platform namespace")]
    ForeignNamespace(String),
    #[error("certificate path {0:?} must point to .der/.pem")]
    BadCertExtension(String),
    #[error("endpoint must be opc.tcp://")]
    NotOpcTcp,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpcUaTagPlan {
    pub endpoint: String,
    pub cert_path: String,
    pub cert_sub_path: String,
    pub tags: Vec<String>,
}

impl OpcUaTagPlan {
    pub fn validate(&self) -> Result<(), OpcUaPlanError> {
        if !self.endpoint.starts_with("opc.tcp://") {
            return Err(OpcUaPlanError::NotOpcTcp);
        }
        for cert in [&self.cert_path, &self.cert_sub_path] {
            let p = cert.to_ascii_lowercase();
            if !(p.ends_with(".der") || p.ends_with(".pem")) {
                return Err(OpcUaPlanError::BadCertExtension(cert.clone()));
            }
        }
        for tag in &self.tags {
            if !tag.starts_with(STATION_TAG_SLUG) {
                return Err(OpcUaPlanError::ForeignNamespace(tag.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn plan() -> OpcUaTagPlan {
        OpcUaTagPlan {
            endpoint: "opc.tcp://192.168.8.10:4840".into(),
            cert_path: "certs/bridge.der".into(),
            cert_sub_path: "certs/machine.der".into(),
            tags: vec![format!("{STATION_TAG_SLUG}line1_count")],
        }
    }

    #[test]
    fn valid_plan_passes() {
        assert!(plan().validate().is_ok());
    }

    #[test]
    fn foreign_ns_and_endpoints_rejected() {
        let mut p = plan();
        p.tags.push("ns=2;s=other.tag".into());
        assert_eq!(
            p.validate(),
            Err(OpcUaPlanError::ForeignNamespace("ns=2;s=other.tag".into()))
        );

        let mut p = plan();
        p.endpoint = "mqtt://10.0.0.1".into();
        assert_eq!(p.validate(), Err(OpcUaPlanError::NotOpcTcp));
    }

    #[test]
    fn cert_paths_constrained() {
        let mut p = plan();
        p.cert_path = "certs/bridge.txt".into();
        assert!(matches!(
            p.validate(),
            Err(OpcUaPlanError::BadCertExtension(_))
        ));
    }
}
