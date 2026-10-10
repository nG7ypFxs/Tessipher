//! Embedded QR decoding with caller-owned memory and no heap allocation.
//!
//! The [`core`] module decodes sampled QR Model 2 matrices. The optional
//! image-detection module scans a borrowed Gray8 region and is enabled by default
//! through the `detect` feature. Disable default features for matrix-only
//! builds; enable `micro-qr` for Micro QR matrix decoding and
//! `micro-qr-detect` for Micro QR image detection.
//!
//! The package is published as `tessipher` while its stable Rust library name
//! remains `qr`, so applications can write `use qr::core::Matrix;`.
//!
//! ```
//! # #[cfg(feature = "detect")]
//! # fn main() -> Result<(), qr::detect::Error> {
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
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "detect"))]
//! # fn main() {}
//! ```
#![no_std]
#![forbid(unsafe_code)]

pub mod core;
#[cfg(feature = "detect")]
pub mod detect;

pub use core::{decode as decode_matrix, Decoded, Matrix, Segment};
#[cfg(feature = "micro-qr")]
pub use core::{
    decode_micro as decode_micro_matrix, MicroDecoded, MicroEcLevel, MicroMatrix, MicroVersion,
};

#[cfg(feature = "detect")]
pub use detect::{
    decode as decode_gray, decode_with_sampling as decode_gray_with_sampling,
    decode_with_sampling_cached as decode_gray_with_sampling_cached,
    decode_with_stats as decode_gray_with_stats, Config, DefaultConfig, Detection, GrayImage, Rect,
    SamplingStrategy, Stats, TrackHint, Tracker, TrackerState, TrackerStats,
};
#[cfg(all(feature = "detect", feature = "micro-qr-detect"))]
pub use detect::{
    decode_micro as decode_micro_gray,
    decode_micro_with_sampling as decode_micro_gray_with_sampling,
    decode_micro_with_stats as decode_micro_gray_with_stats, MicroDetection,
};

/// Scratch requirements are independent of configured image dimensions.
#[cfg(feature = "detect")]
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
#[cfg(feature = "detect")]
pub fn memory_requirements<const W: usize, const H: usize>(
    config: &Config<W, H>,
) -> Result<MemoryRequirements, detect::Error> {
    Ok(MemoryRequirements {
        core_bytes: core::SCRATCH_BYTES,
        detector_bytes: config.scratch_bytes()?,
        alignment: 1,
    })
}
