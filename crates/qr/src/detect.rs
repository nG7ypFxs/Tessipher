//! Bounded Gray8 detector with borrowed input and no internal allocation.
//! Fast optionally uses caller storage for two-bit local pixel classifications.
//! Global midrange and local sampled-mean thresholds, each in normal/inverted polarity.
//! Fast also tries nearby global thresholds before local evaluation.
//!
//! [`GrayImage`] borrows a rectangular ROI, including a parent frame's row stride.
//! [`Config`] sets independent image maxima and finder/candidate/sampling and pixel-read budgets.
//! [`decode`] returns the first decoded symbol. [`decode_with_stats`] also exposes
//! counters after failure. No call enumerates every symbol in an image.
//! Feature `micro-qr-detect` adds separate single-finder `decode_micro` APIs.
//!
//! ```
//! use tessipher::detect::{Config, GrayImage};
//! let pixels = [255; 8];
//! let roi = GrayImage::new(&pixels, 2, 2, 6)?;
//! assert_eq!(roi.pixel(1, 1), Some(255));
//! let config = Config::<640, 240>::default();
//! assert_eq!(config.scratch_bytes()?, 4557);
//! # Ok::<(), tessipher::detect::Error>(())
//! ```
#[cfg(feature = "micro-qr-detect")]
use crate::core::MicroDecoded;
use crate::core::{Decoded, Segment, MAX_MATRIX_BYTES};

#[cfg(feature = "micro-qr-detect")]
mod micro;
mod region;
mod sampling;
mod scan;
mod threshold;
mod tracking;

#[cfg(feature = "micro-qr-detect")]
pub use micro::{decode_micro, decode_micro_with_sampling, decode_micro_with_stats};
use region::decode_region;
use threshold::PixelBudget;
pub use tracking::{TrackHint, Tracker, TrackerState, TrackerStats};

/// Required detector scratch alignment, in bytes (1).
pub const SCRATCH_ALIGN: usize = 1;
const FINDER_BYTES: usize = 20;
/// Image validation, bounded search, or matrix decoding failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Zero dimensions, insufficient pixels, invalid stride, or size arithmetic overflow.
    InvalidImage,
    /// The ROI exceeds the configured compile-time dimensions.
    ImageTooLarge,
    /// An image maximum or runtime setting is outside its allowed range.
    InvalidConfig,
    /// Detector scratch is shorter than [`Config::scratch_bytes`].
    ScratchTooSmall,
    /// Finder observations were discarded for lack of capacity and no symbol decoded.
    FinderCapacity,
    /// The shared finder-triple budget was exhausted.
    CandidateBudget,
    /// The shared matrix-sampling budget was exhausted.
    RetryBudget,
    /// The shared pixel-read budget was exhausted before another input read.
    PixelBudget,
    /// No symbol decoded and no more specific failure was retained.
    NotFound,
    /// Core validation, capacity, or decoding failure.
    Decode(crate::core::Error),
}
/// Borrowed ROI. `data` begins at the first pixel of the ROI, not the parent frame.
#[derive(Clone, Copy)]
pub struct GrayImage<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}
impl<'a> GrayImage<'a> {
    /// Validates a borrowed Gray8 ROI without copying pixels.
    ///
    /// Width and height must be nonzero, stride must be at least width, and data
    /// must contain `(height - 1) * stride + width` bytes. Arithmetic overflow is
    /// rejected. Padding after the last row is unnecessary. `data[0]` must be the
    /// ROI's top-left pixel; retain the parent frame stride when selecting a ROI.
    /// Pixel values range from 0 (black) to 255 (white).
    pub fn new(data: &'a [u8], width: usize, height: usize, stride: usize) -> Result<Self, Error> {
        if width == 0 || height == 0 || stride < width {
            return Err(Error::InvalidImage);
        }
        let needed = (height - 1)
            .checked_mul(stride)
            .and_then(|x| x.checked_add(width))
            .ok_or(Error::InvalidImage)?;
        if data.len() < needed {
            return Err(Error::InvalidImage);
        }
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }
    /// Returns the ROI width in pixels.
    pub fn width(self) -> usize {
        self.width
    }
    /// Returns the ROI height in pixels.
    pub fn height(self) -> usize {
        self.height
    }
    /// Returns the byte distance between row starts.
    pub fn stride(self) -> usize {
        self.stride
    }
    /// Returns a pixel at ROI-relative `(x, y)`, or `None` outside the ROI.
    pub fn pixel(self, x: usize, y: usize) -> Option<u8> {
        if x < self.width && y < self.height {
            Some(self.at(x, y))
        } else {
            None
        }
    }
    /// Borrows a validated subregion, preserving the parent stride and requiring
    /// no padding after its final row. Coordinates are relative to this image.
    pub fn sub_roi(self, rect: Rect) -> Result<Self, Error> {
        let right = rect.x.checked_add(rect.width).ok_or(Error::InvalidImage)?;
        let bottom = rect.y.checked_add(rect.height).ok_or(Error::InvalidImage)?;
        if rect.width == 0 || rect.height == 0 || right > self.width || bottom > self.height {
            return Err(Error::InvalidImage);
        }
        let start = rect
            .y
            .checked_mul(self.stride)
            .and_then(|n| n.checked_add(rect.x))
            .ok_or(Error::InvalidImage)?;
        Self::new(&self.data[start..], rect.width, rect.height, self.stride)
    }
    fn at(self, x: usize, y: usize) -> u8 {
        self.data[y * self.stride + x]
    }
}
/// Integer rectangle relative to the input image's top-left pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left coordinate in pixels.
    pub x: usize,
    /// Top coordinate in pixels.
    pub y: usize,
    /// Nonzero width in pixels.
    pub width: usize,
    /// Nonzero height in pixels.
    pub height: usize,
}

/// Sampling policy for each candidate projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SamplingStrategy {
    /// Sample each module's center once (the legacy behavior).
    #[default]
    Center,
    /// After a complete center sample fails core content decoding, retry with five
    /// votes: the center and offsets of 0.25 module up, down, left, and right.
    /// All five points are read, including repeated pixels; three dark votes win.
    CenterThenMajority5,
}

/// Detector execution policy. Model 2 APIs default to [`Self::Direct`].
/// The explicit five-row cached APIs use their own row-cache execution path.
///
/// All modes support the same minimal caller-owned buffers and allocate no memory.
/// Fast can use extra caller scratch for exact local threshold memoization. Faster
/// modes may read fewer pixels and select a different first symbol. Finite work
/// budgets can therefore produce different results between modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetectionMode {
    /// Reuse matrix scratch for local column summaries and prune alignment work.
    /// Images wider than the scratch can hold use direct local evaluation.
    #[default]
    Direct,
    /// Direct with at least three pixels between scanned rows, followed by
    /// the configured row spacing after complete recognition failure.
    /// Before local evaluation, also try global thresholds one sixteenth of the
    /// ROI intensity span below and above its midrange, in both polarities.
    /// Each nearby-threshold pass probes at most two matrix samplings.
    /// Both searches and all thresholds share budgets; terminal errors never trigger fallback.
    /// Extra scratch from [`Config::scratch_bytes_for_mode`] enables a two-bit
    /// per-pixel local-threshold memo; minimal scratch keeps the bounded path.
    Fast,
}

/// Default maximum actual input pixel reads across a complete decode call.
/// Independent of image dimensions; includes min/max, repeated and local reads.
pub const DEFAULT_MAX_PIXEL_READS: usize = 64_000_000;

/// Image maxima are independent const parameters. They do not change minimal scratch size.
/// The optional Fast extension depends on the actual image dimensions.
///
/// Both maxima must be in 1..=32768. Defaults are 640 by 640 pixels, 32 finder
/// records, 4096 candidates, 256 sampling attempts, row step 1, and local radius 24.
/// Settings are validated by [`Self::scratch_bytes`] and the decode entry points.
#[derive(Clone, Copy, Debug)]
pub struct Config<const MAX_WIDTH: usize = 640, const MAX_HEIGHT: usize = 640> {
    /// Finder-record capacity per pass (3..=128); default 32.
    pub max_finders: usize,
    /// Total triples examined across all threshold/polarity passes (1..=1_000_000).
    /// Default 4096; geometric rejection still consumes a candidate.
    pub max_candidates: usize,
    /// Total matrix-sampling attempts across all passes (1..=65536); default 256.
    /// Includes samples rejected geometrically. Core transpose retries do not count.
    pub max_retries: usize,
    /// Row scan increment in pixels (1..=MAX_HEIGHT); default 1.
    /// One preserves the best small-symbol recall.
    pub row_step: usize,
    /// Half-width of the sampled local-threshold window in pixels (4..=128); default 24.
    pub local_radius: usize,
    /// Actual input pixel reads across all passes (1..=usize::MAX); default
    /// [`DEFAULT_MAX_PIXEL_READS`]. Includes initial min/max and repeated reads.
    /// Uncached sampled local decisions cost up to 26 reads, including repeated
    /// clamped samples. Direct/Fast may stop once exact sum bounds decide the
    /// result. Column summaries and caches charge their actual input reads.
    /// This bounds pixel work, not total CPU cycles or caller buffer initialization.
    pub max_pixel_reads: usize,
}
impl<const W: usize, const H: usize> Default for Config<W, H> {
    fn default() -> Self {
        Self {
            max_finders: 32,
            max_candidates: 4096,
            max_retries: 256,
            row_step: 1,
            local_radius: 24,
            max_pixel_reads: DEFAULT_MAX_PIXEL_READS,
        }
    }
}
impl<const W: usize, const H: usize> Config<W, H> {
    /// Compile-time maximum ROI width, in pixels.
    pub const MAX_WIDTH: usize = W;
    /// Compile-time maximum ROI height, in pixels.
    pub const MAX_HEIGHT: usize = H;
    /// Returns `tessipher::core::MAX_MATRIX_BYTES + 20 * max_finders` after validation.
    ///
    /// Returns [`Error::InvalidConfig`] for any invalid setting or image maximum.
    /// The returned allocation is byte-aligned and may be reused between calls.
    pub fn scratch_bytes(&self) -> Result<usize, Error> {
        self.validate()?;
        Ok(MAX_MATRIX_BYTES + self.max_finders * FINDER_BYTES)
    }
    /// Detector scratch including optional Fast exact local-threshold memoization.
    ///
    /// Fast adds `ceil(width * height / 4)` bytes to [`Self::scratch_bytes`].
    /// Direct returns the minimal size. Nonzero dimensions must fit
    /// the validated configuration, otherwise [`Error::InvalidConfig`] is returned.
    /// Supplying only the minimal size to Fast remains supported. The memo is
    /// initialized only when a local-threshold pass is needed. Cached decisions
    /// consume no input reads, so finite pixel budgets can change decode results.
    pub fn scratch_bytes_for_mode(
        &self,
        mode: DetectionMode,
        width: usize,
        height: usize,
    ) -> Result<usize, Error> {
        let base = self.scratch_bytes()?;
        if width == 0 || height == 0 || width > W || height > H {
            return Err(Error::InvalidConfig);
        }
        Ok(base
            + if mode == DetectionMode::Fast {
                (width * height).div_ceil(4)
            } else {
                0
            })
    }
    /// Bytes for an optional five-row Gray8 cache in caller-owned internal RAM.
    /// Alignment is one; `width` must fit the validated configuration.
    pub fn local_cache_bytes(&self, width: usize) -> Result<usize, Error> {
        self.validate()?;
        if width == 0 || width > W {
            return Err(Error::InvalidConfig);
        }
        width.checked_mul(5).ok_or(Error::InvalidConfig)
    }
    fn validate(&self) -> Result<(), Error> {
        if W == 0
            || H == 0
            || W > 32768
            || H > 32768
            || !(3..=128).contains(&self.max_finders)
            || self.max_candidates == 0
            || self.max_candidates > 1_000_000
            || self.max_pixel_reads == 0
            || self.max_retries == 0
            || self.max_retries > 65536
            || self.row_step == 0
            || self.row_step > H
            || !(4..=128).contains(&self.local_radius)
        {
            Err(Error::InvalidConfig)
        } else {
            Ok(())
        }
    }
}
/// Configuration with independent 640-pixel width and height maxima.
pub type DefaultConfig = Config<640, 640>;
/// ROI-relative image coordinate, with x rightward and y downward.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Point {
    /// Horizontal coordinate in pixels.
    pub x: f32,
    /// Vertical coordinate in pixels.
    pub y: f32,
}
impl Point {
    fn add(self, p: Self) -> Self {
        Self {
            x: self.x + p.x,
            y: self.y + p.y,
        }
    }
    fn sub(self, p: Self) -> Self {
        Self {
            x: self.x - p.x,
            y: self.y - p.y,
        }
    }
    fn scale(self, t: f32) -> Self {
        Self {
            x: self.x * t,
            y: self.y * t,
        }
    }
    fn dot(self, p: Self) -> f32 {
        self.x * p.x + self.y * p.y
    }
    fn norm(self) -> f32 {
        sqrt(self.dot(self))
    }
    fn cross(self, p: Self) -> f32 {
        self.x * p.y - self.y * p.x
    }
}
/// Work counters accumulated across the complete decode call.
///
/// Early validation failures leave all counters at zero. Pixel reads include
/// the initial min/max scan, including on low-contrast images or budget failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Actual input pixel reads, including repeats and cache fills, excluding
    /// cache hits in internal RAM; never exceeds max_pixel_reads.
    pub pixel_reads: usize,
    /// Rows visited by finder scanning, summed across passes.
    pub rows: usize,
    /// Run-ratio observations passing both cross-checks, before 7×7 pattern validation.
    pub finder_observations: usize,
    /// Finder triples examined, including those rejected before sampling.
    pub candidates: usize,
    /// Matrix-sampling attempts, including geometrically rejected samples.
    /// A core transpose retry reuses one sample and does not increment this count.
    pub retries: usize,
    /// Threshold/polarity passes entered. At most 4 in Direct or 16 in Fast per region;
    /// tracking can search two regions, doubling these bounds.
    pub passes: usize,
}
/// First successfully decoded symbol and the sampling geometry used to find it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    /// Decoded payload metadata; only its output prefixes are valid.
    pub decoded: Decoded,
    /// ROI-relative projections of grid boundaries `(0,0)`, `(side,0)`,
    /// `(side,side)`, and `(0,side)`, in sampling order.
    /// Core transpose decoding leaves this order unchanged; for a mirrored symbol,
    /// logical top-right and bottom-left correspond to indices 3 and 1.
    /// There is no mirrored flag in the result.
    pub corners: [Point; 4],
    /// Whether the successful threshold pass used inverted polarity.
    pub inverted: bool,
    /// Whether the successful pass used local sampled-mean thresholds.
    pub local_threshold: bool,
    /// Counters at the point of successful decoding.
    pub stats: Stats,
}
/// First successfully decoded Micro QR symbol and its sampling geometry.
#[cfg(feature = "micro-qr-detect")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MicroDetection {
    /// Decoded Micro QR payload metadata.
    pub decoded: MicroDecoded,
    /// ROI-relative projections of the four sampled-grid boundaries.
    pub corners: [Point; 4],
    /// Whether the successful threshold pass used inverted polarity.
    pub inverted: bool,
    /// Whether the successful pass used local sampled-mean thresholds.
    pub local_threshold: bool,
    /// Counters at the point of successful decoding.
    pub stats: Stats,
}

fn sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fc00000);
    for _ in 0..5 {
        y = (y + x / y) * 0.5;
    }
    y
}

/// Decodes at most one symbol in [`DetectionMode::Direct`], using deterministic
/// threshold/polarity/triple/version priority.
/// All budgets are shared across the complete call. Thresholds are tried in order:
/// global normal, global inverted, local normal, local inverted.
///
/// Supply detector scratch from [`Config::scratch_bytes`] and core scratch of at
/// least [`crate::core::SCRATCH_BYTES`] bytes. All writable storage belongs to the caller
/// and can be reused. On success, read only the payload and segment prefixes named
/// by [`Detection::decoded`]; on failure, writable buffer contents are unspecified.
/// Use [`decode_with_stats`] when failure counters are needed.
///
/// Rotation, perspective, uneven lighting, and mirrors are supported by bounded
/// heuristics; recognition under arbitrary distortion is not guaranteed.
pub fn decode<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<Detection, Error> {
    decode_with_stats(
        image,
        config,
        scratch,
        core_scratch,
        output,
        segments,
        &mut Stats::default(),
    )
}
/// Like [`decode`], retaining work counters on both success and failure.
///
/// Resets `stats` before validation. On success, it equals [`Detection::stats`].
/// On error, payload, segment, and scratch contents remain unspecified.
#[allow(clippy::too_many_arguments)]
pub fn decode_with_stats<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
) -> Result<Detection, Error> {
    decode_with_sampling(
        image,
        config,
        SamplingStrategy::Center,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
    )
}
/// Like [`decode_with_stats`], with an opt-in sampling policy.
///
/// A majority retry uses the same projection and threshold, consumes one retry,
/// and shares all budgets. Geometry, input, capacity, and budget errors never
/// trigger fallback. A center success performs no extra reads or retries.
#[allow(clippy::too_many_arguments)]
pub fn decode_with_sampling<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
) -> Result<Detection, Error> {
    decode_sampling_inner(
        image,
        config,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        None,
        DetectionMode::Direct,
    )
}
/// Decodes one Model 2 symbol with an explicit execution and sampling policy.
///
/// Both modes support the same minimal caller-owned buffers; Fast can use extra
/// scratch for local-threshold memoization. Both modes reuse matrix scratch for
/// column summaries only during local finder scans; widths above `MAX_MATRIX_BYTES / 4` use direct local threshold evaluation.
/// Fast first scans every `max(config.row_step, 3)` rows, then retries using the
/// configured row spacing after complete recognition failure. All passes share
/// pixel, candidate and sampling budgets; capacity and budget failures are final.
/// Finite budgets and first-symbol selection can differ between modes.
/// `stats` resets before validation and is retained on failure.
#[allow(clippy::too_many_arguments)]
pub fn decode_with_mode<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    mode: DetectionMode,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
) -> Result<Detection, Error> {
    decode_sampling_inner(
        image,
        config,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        None,
        mode,
    )
}

/// Uses five caller-owned Gray8 rows for local passes with the dedicated row-cache path.
/// Put `cache` in internal RAM; it needs [`Config::local_cache_bytes`] bytes.
/// Only horizontal finder scans fill rows. Irregular helpers reuse hits and
/// read misses directly without replacing cached rows. Tags reset on every call;
/// excess bytes remain untouched. Cache fills count as
/// input reads, hits do not. The same finite budget may produce a different result
/// because a miss fetches a complete row, including pixels not yet requested.
#[allow(clippy::too_many_arguments)]
pub fn decode_with_sampling_cached<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    cache: &mut [u8],
) -> Result<Detection, Error> {
    decode_sampling_inner(
        image,
        config,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        Some(cache),
        DetectionMode::Direct,
    )
}
#[allow(clippy::too_many_arguments)]
fn decode_sampling_inner<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    cache: Option<&mut [u8]>,
    mode: DetectionMode,
) -> Result<Detection, Error> {
    *stats = Stats::default();
    validate_input(
        image,
        config,
        scratch,
        core_scratch,
        crate::core::SCRATCH_BYTES,
    )?;
    if let Some(bytes) = cache.as_ref() {
        if bytes.len() < config.local_cache_bytes(image.width)? {
            return Err(Error::ScratchTooSmall);
        }
    }
    let budget = PixelBudget::new(config.max_pixel_reads);
    let result = decode_region(
        image,
        config,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        &budget,
        None,
        cache,
        mode,
    );
    stats.pixel_reads = budget.used.get();
    result.map(|mut found| {
        found.stats = *stats;
        found
    })
}

fn validate_input<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &[u8],
    core_scratch: &[u8],
    required_core_bytes: usize,
) -> Result<(), Error> {
    let needed = config.scratch_bytes()?;
    if image.width > W || image.height > H {
        return Err(Error::ImageTooLarge);
    }
    if scratch.len() < needed {
        return Err(Error::ScratchTooSmall);
    }
    if core_scratch.len() < required_core_bytes {
        return Err(Error::Decode(crate::core::Error::ScratchTooSmall));
    }
    Ok(())
}
