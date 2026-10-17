//! Device target wiring — compiled only on esp targets
//! (`cfg(target_arch = "xtensa")` / `cfg(target_arch = "riscv32")`).
//! Host CI never resolves this module; the vendor crates are target-gated
//! in Cargo.toml.
//!
//! The first real firmware slice lives on hardware owners' bring-up; the
//! pure-DSP units above (csi / dsp / velocity / rp / audits) are the same
//! code that runs here.

pub const FIRMWARE_TRIPLE_XTENSA: &[&str] = &["xtensa-esp32s3-none-elf", "xtensa-esp32c5-none-elf"];
pub const FIRMWARE_TRIPLE_RISCV: &[&str] = &["riscv32imac-esp-none-elf"];
pub const REQUIRED_CHIP_CAPS: &[&str] = &["csi", "esp-now", "1khz"];

pub fn target_family() -> &'static str {
    #[cfg(target_arch = "xtensa")]
    {
        "xtensa"
    }
    #[cfg(target_arch = "riscv32")]
    {
        "riscv32"
    }
}
