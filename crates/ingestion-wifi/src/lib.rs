//! Shado Wi-Fi CSI ingestion (docs/05).
//!
//! Fixture-first build, matching docs/14 §2: parsing, phase sanitization and
//! the sliding-window feature pipeline are pure and deterministic here; the
//! UDP/serial/pcap production transports implement [`sources::CsiSource`] in
//! the hardware phase. Alias-parity: the same types/trait structure keeps
//! `RawCsiRecord` and window features byte-identical when real nodes arrive.

pub mod error;
pub mod features;
pub mod npy;
pub mod record;
pub mod sanitize;
pub mod sources;
pub mod window;

pub use error::CsiError;
pub use features::{
    WindowConfig, WindowFeatures, compute_window_features, estimate_mover_count_from_band,
    motion_variance, presence_from_energy,
};
pub use npy::{npy_f32_bytes_to_vec, read_npy_f32, to_npy_f32_bytes, write_npy_f32};
pub use record::{DecodedPacket, MacAddr, RawCsiRecord, parse_iq8};
pub use sanitize::{
    SanitizedWindow, median_f32, sanitize_packets, unwrap_and_detrend_across_subcarriers,
    unwrap_over_time,
};
pub use sources::{
    CsiHandle, CsiSource, ReplayCsiSource, SimulatedMover, SyntheticCsiConfig, SyntheticCsiSource,
    generate_synthetic_records,
};
pub use window::{CsiWindowProcessor, WindowSpan, features_to_wifi_tick};
