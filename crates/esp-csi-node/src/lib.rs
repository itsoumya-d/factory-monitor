//! COTS Wi-Fi CSI node firmware components (docs/17 Layer A). Pure-DSP and
//! guardrail units are no_std-safe and unit-tested on the host; device
//! wiring is target-gated per chip family.

#![cfg_attr(not(test), no_std)]

// Memory allocations for no_std device builds (std/test builds get the
// std prelude for the same names).
#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
pub use alloc::{format, string::String, vec, vec::Vec};

pub mod audits;
pub mod capability;
pub mod csi;
pub mod dsp;
pub mod phase_sync;
pub mod rp;
pub mod velocity;

#[cfg(target_arch = "xtensa")]
pub mod device;
