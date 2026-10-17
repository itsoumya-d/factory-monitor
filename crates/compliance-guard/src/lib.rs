//! Compliance-by-architecture: EU AI Act Art 5(1)(f) emotion lock,
//! Annex III safeguards, subject-level ε-DP, and the DiagnosableNode
//! contract every perceivable node must satisfy.

pub mod aggregation;
pub mod annex3;
pub mod article5;
pub mod dp;
pub mod node_diag;

pub use node_diag::DiagnosableNode;
pub mod validate;
