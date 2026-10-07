//! Embedded QR integration facade with caller-owned memory and no heap allocation.
//!
//! [`core`] decodes sampled QR Model 2 matrices independently of image processing.
//! [`detect`] scans borrowed Gray8 regions and returns the first decoded symbol.
//! Use [`memory_requirements`] to size scratch, then [`decode_matrix`] or
//! [`decode_gray`] with reusable caller-owned payload and segment buffers.
//! On success, read only the lengths reported by [`Decoded`]; writable buffers
//! have unspecified contents after failure.
//!
//! ```
//! use qr::{Config, GrayImage, Segment};
//! let config = Config::<8, 8>::default();
//! let required = qr::memory_requirements(&config)?;
//! assert_eq!(required.core_bytes, qr::core::SCRATCH_BYTES);
//! assert_eq!(required.detector_bytes, 4557);
//! let pixels = [255; 64];
//! let image = GrayImage::new(&pixels, 8, 8, 8)?;
//! let mut detector_scratch = [0; 4557];
//! let mut core_scratch = [0; qr::core::SCRATCH_BYTES];
//! let mut payload = [0; 32];
//! let mut segments = [Segment::EMPTY; 4];
//! let result = qr::decode_gray(image, &config, &mut detector_scratch,
//!     &mut core_scratch, &mut payload, &mut segments);
//! assert_eq!(result, Err(qr::detect::Error::NotFound));
//! # Ok::<(), qr::detect::Error>(())
//! ```
#![no_std]
#![forbid(unsafe_code)]
pub use qr_core as core;
pub use qr_core::{decode as decode_matrix, Decoded, Matrix, Segment};
pub use qr_detect as detect;
pub use qr_detect::{decode as decode_gray, Config, DefaultConfig, Detection, GrayImage};
/// Scratch requirements are independent of configured image dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRequirements {
    /// Core scratch bytes for matrix decoding.
    pub core_bytes: usize,
    /// Detector scratch bytes for the validated finder capacity.
    pub detector_bytes: usize,
    /// Required alignment for both byte scratch buffers.
    pub alignment: usize,
}
/// Returns scratch requirements after validating all configuration settings.
///
/// Does not include payload or segment storage. Returns [`detect::Error::InvalidConfig`]
/// for invalid budgets or compile-time dimensions.
pub fn memory_requirements<const W: usize, const H: usize>(
    config: &Config<W, H>,
) -> Result<MemoryRequirements, detect::Error> {
    Ok(MemoryRequirements {
        core_bytes: core::SCRATCH_BYTES,
        detector_bytes: config.scratch_bytes()?,
        alignment: 1,
    })
}
