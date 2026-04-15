//! Embedded QR integration facade. Core and detector remain independently usable.
#![no_std]
#![forbid(unsafe_code)]
pub use qr_core as core;
pub use qr_core::{decode as decode_matrix, Decoded, Matrix, Segment};
pub use qr_detect as detect;
pub use qr_detect::{decode as decode_gray, Config, DefaultConfig, Detection, GrayImage};
/// Scratch requirements are independent of configured image dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRequirements {
    pub core_bytes: usize,
    pub detector_bytes: usize,
    pub alignment: usize,
}
pub fn memory_requirements<const W: usize, const H: usize>(
    config: &Config<W, H>,
) -> Result<MemoryRequirements, detect::Error> {
    Ok(MemoryRequirements {
        core_bytes: core::SCRATCH_BYTES,
        detector_bytes: config.scratch_bytes()?,
        alignment: 1,
    })
}
