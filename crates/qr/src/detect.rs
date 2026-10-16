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
use crate::core::{Decoded, Matrix, Segment, MAX_MATRIX_BYTES};
#[cfg(feature = "micro-qr-detect")]
use crate::core::{MicroDecoded, MicroMatrix};
use core::cell::{Cell, RefCell};

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
#[derive(Clone, Copy, Default)]
struct Finder {
    p: Point,
    pitch_x: f32,
    pitch_y: f32,
    hits: u32,
}
fn read_finder(data: &[u8], i: usize) -> Finder {
    let p = i * FINDER_BYTES;
    let f = |n| {
        f32::from_le_bytes([
            data[p + n],
            data[p + n + 1],
            data[p + n + 2],
            data[p + n + 3],
        ])
    };
    Finder {
        p: Point { x: f(0), y: f(4) },
        pitch_x: f(8),
        pitch_y: f(12),
        hits: u32::from_le_bytes([data[p + 16], data[p + 17], data[p + 18], data[p + 19]]),
    }
}
fn write_finder(data: &mut [u8], i: usize, f: Finder) {
    let p = i * FINDER_BYTES;
    data[p..p + 4].copy_from_slice(&f.p.x.to_le_bytes());
    data[p + 4..p + 8].copy_from_slice(&f.p.y.to_le_bytes());
    data[p + 8..p + 12].copy_from_slice(&f.pitch_x.to_le_bytes());
    data[p + 12..p + 16].copy_from_slice(&f.pitch_y.to_le_bytes());
    data[p + 16..p + 20].copy_from_slice(&f.hits.to_le_bytes());
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
/// One counter shared by min/max and every detector pass/helper.
struct PixelBudget {
    limit: usize,
    used: Cell<usize>,
}
impl PixelBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            used: Cell::new(0),
        }
    }
    fn read(&self, image: GrayImage<'_>, x: usize, y: usize) -> Result<u8, Error> {
        let used = self.used.get();
        if used >= self.limit {
            return Err(Error::PixelBudget);
        }
        // Check before reading; used + 1 is safe even when limit == usize::MAX.
        self.used.set(used + 1);
        Ok(image.at(x, y))
    }
}
// Tags are scoped to one decode region. Incomplete fills never become valid.
struct RowCache<'a> {
    bytes: &'a mut [u8],
    tags: [Option<usize>; 5],
    order: [usize; 5],
}
impl<'a> RowCache<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self {
            bytes,
            tags: [None; 5],
            order: [0, 1, 2, 3, 4],
        }
    }
    fn read(
        &mut self,
        image: GrayImage<'_>,
        budget: &PixelBudget,
        x: usize,
        y: usize,
        fill: bool,
    ) -> Result<u8, Error> {
        let slot = match self.tags.iter().position(|tag| *tag == Some(y)) {
            Some(slot) => slot,
            None if !fill => return budget.read(image, x, y),
            None => {
                let slot = self.order[0];
                self.tags[slot] = None;
                let start = slot * image.width;
                for col in 0..image.width {
                    self.bytes[start + col] = budget.read(image, col, y)?;
                }
                self.tags[slot] = Some(y);
                slot
            }
        };
        if fill {
            let position = self.order.iter().position(|&n| n == slot).unwrap();
            self.order[position..].rotate_left(1);
        }
        Ok(self.bytes[slot * image.width + x])
    }
}
// Byte-packed summaries reuse matrix scratch only while scanning. The matrix
// is not sampled until the scan has finished, so their lifetimes never overlap.
// No alignment beyond the detector's byte-aligned scratch is required.
struct ColumnSummary<'a> {
    bytes: &'a mut [u8],
}
impl<'a> ColumnSummary<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes }
    }
    fn fill<const OPT: bool>(&mut self, bin: Binary<'_, OPT>, y: usize) -> Result<(), Error> {
        let used = bin.budget.used.get();
        let reads = bin.image.width * 5;
        // A complete fill has no helper callbacks or fallible reads. Charge
        // it once and zip validated rows so the compiler can process columns
        // in parallel. Keep the scalar path for exact partial-budget behavior.
        if OPT && reads <= bin.budget.limit - used {
            let rows: [&[u8]; 5] = core::array::from_fn(|i| {
                let py = (y as i32 + (i as i32 - 2) * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let start = py * bin.image.stride;
                &bin.image.data[start..start + bin.image.width]
            });
            bin.budget.used.set(used + reads);
            for (out, ((((&a, &b), &c), &d), &e)) in self.bytes.chunks_exact_mut(4).zip(
                rows[0]
                    .iter()
                    .zip(rows[1])
                    .zip(rows[2])
                    .zip(rows[3])
                    .zip(rows[4]),
            ) {
                let sum = a as u16 + b as u16 + c as u16 + d as u16 + e as u16;
                out[..2].copy_from_slice(&sum.to_le_bytes());
                out[2] = a.min(b).min(c).min(d).min(e);
                out[3] = a.max(b).max(c).max(d).max(e);
            }
            return Ok(());
        }
        for x in 0..bin.image.width {
            let mut sum = 0u16;
            let mut min = 255;
            let mut max = 0;
            for j in -2i32..=2 {
                // Divide after multiplication: odd radii use truncation toward
                // zero, exactly as in the direct sampled threshold.
                let py = (y as i32 + j * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let value = bin.budget.read(bin.image, x, py)?;
                sum += value as u16;
                min = min.min(value);
                max = max.max(value);
            }
            let start = x * 4;
            self.bytes[start..start + 2].copy_from_slice(&sum.to_le_bytes());
            self.bytes[start + 2] = min;
            self.bytes[start + 3] = max;
        }
        Ok(())
    }
    #[inline]
    fn dark<const OPT: bool>(
        &self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<bool, Error> {
        if let Some(dark) = bin.memo_get(x, y) {
            return Ok(dark ^ bin.inverted);
        }
        let pixel = bin.budget.read(bin.image, x, y)? as u32;
        let records: [u32; 5] = core::array::from_fn(|i| {
            let px = (x as i32 + (i as i32 - 2) * bin.radius as i32 / 2)
                .clamp(0, bin.image.width as i32 - 1) as usize;
            let start = px * 4;
            let bytes = &self.bytes[start..start + 4];
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        });
        let sum: u32 = records.iter().map(|record| record & 0xffff).sum();
        let mean_dark = pixel * 25 < sum;
        let fallback_dark = pixel <= bin.threshold as u32;
        // Only contrast selects between these two decisions; when they agree,
        // its value cannot affect classification.
        let dark = if mean_dark == fallback_dark {
            mean_dark
        } else {
            let mut min = 255u8;
            let mut max = 0u8;
            for record in records {
                min = min.min((record >> 16) as u8);
                max = max.max((record >> 24) as u8);
            }
            if max - min < 12 {
                fallback_dark
            } else {
                mean_dark
            }
        };
        bin.memo_set(x, y, dark);
        Ok(dark ^ bin.inverted)
    }
}
#[derive(Clone, Copy)]
struct Binary<'a, const OPT: bool = false> {
    image: GrayImage<'a>,
    budget: &'a PixelBudget,
    cache: Option<&'a RefCell<RowCache<'a>>>,
    // Two bits per ROI pixel: 0 unknown, 1 dark, 2 light. Store un-inverted
    // decisions only after a complete successful threshold calculation.
    memo: Option<&'a [Cell<u8>]>,
    fill_cache: bool,
    threshold: u8,
    inverted: bool,
    local: bool,
    // Conservative bounds on every pixel in this ROI, from its min/max pass.
    range: [u8; 2],
    radius: usize,
}
impl<const OPT: bool> Binary<'_, OPT> {
    #[inline]
    fn memo_get(self, x: usize, y: usize) -> Option<bool> {
        let bytes = self.memo?;
        let index = y * self.image.width + x;
        let value = (bytes[index / 4].get() >> ((index % 4) * 2)) & 3;
        (value != 0).then_some(value == 1)
    }
    #[inline]
    fn memo_set(self, x: usize, y: usize, dark: bool) {
        if let Some(bytes) = self.memo {
            let index = y * self.image.width + x;
            let byte = &bytes[index / 4];
            let shift = (index % 4) * 2;
            byte.set((byte.get() & !(3 << shift)) | ((if dark { 1 } else { 2 }) << shift));
        }
    }

    #[inline]
    fn read(self, x: usize, y: usize) -> Result<u8, Error> {
        if self.local {
            if let Some(cache) = self.cache {
                return cache
                    .borrow_mut()
                    .read(self.image, self.budget, x, y, self.fill_cache);
            }
        }
        self.budget.read(self.image, x, y)
    }
    #[inline]
    fn dark(self, x: usize, y: usize) -> Result<bool, Error> {
        if OPT && self.local {
            if let Some(dark) = self.memo_get(x, y) {
                return Ok(dark ^ self.inverted);
            }
        }
        let pixel = self.read(x, y)? as u32;
        let dark = if !self.local {
            pixel <= self.threshold as u32
        } else {
            'threshold: {
                // A fixed 5x5 sample window with no integral-image buffer.
                let mut sum = 0u32;
                let mut min = 255u8;
                let mut max = 0u8;
                for j in -2i32..=2 {
                    for i in -2i32..=2 {
                        let px = (x as i32 + i * self.radius as i32 / 2)
                            .clamp(0, self.image.width as i32 - 1)
                            as usize;
                        let py = (y as i32 + j * self.radius as i32 / 2)
                            .clamp(0, self.image.height as i32 - 1)
                            as usize;
                        let v = self.read(px, py)?;
                        sum += v as u32;
                        min = min.min(v);
                        max = max.max(v);
                    }
                    // Once contrast reaches 12 it cannot fall again. The sum can
                    // grow within the image extrema for each unread sample, so these
                    // bounds prove the final strict mean comparison exactly.
                    if OPT && max - min >= 12 {
                        let target = pixel * 25;
                        let remaining = (2 - j) as u32 * 5;
                        if sum + remaining * self.range[0] as u32 > target {
                            break 'threshold true;
                        }
                        if sum + remaining * self.range[1] as u32 <= target {
                            break 'threshold false;
                        }
                    }
                }
                if max - min < 12 {
                    pixel <= self.threshold as u32
                } else {
                    pixel * 25 < sum
                }
            }
        };
        if OPT && self.local {
            self.memo_set(x, y, dark);
        }
        Ok(dark ^ self.inverted)
    }

    #[inline]
    fn global_point(self, p: Point) -> Result<Option<bool>, Error> {
        if !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.0
            || p.y < 0.0
            || p.x >= self.image.width as f32
            || p.y >= self.image.height as f32
        {
            Ok(None)
        } else {
            self.budget
                .read(self.image, p.x as usize, p.y as usize)
                .map(|value| Some((value <= self.threshold) ^ self.inverted))
        }
    }
    #[inline]
    fn point(self, p: Point) -> Result<Option<bool>, Error> {
        if !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.0
            || p.y < 0.0
            || p.x >= self.image.width as f32
            || p.y >= self.image.height as f32
        {
            Ok(None)
        } else {
            Ok(Some(self.dark(p.x as usize, p.y as usize)?))
        }
    }
}
fn ratio(r: &[usize; 5]) -> Option<f32> {
    let sum = r.iter().sum::<usize>();
    if sum < 7 || r.contains(&0) {
        return None;
    }
    let m = sum as f32 / 7.0;
    let tolerance = m * 0.85;
    for (i, &n) in r.iter().enumerate() {
        let expected = m * if i == 2 { 3.0 } else { 1.0 };
        if (n as f32 - expected).abs() > tolerance * if i == 2 { 1.6 } else { 1.0 } {
            return None;
        }
    }
    Some(m)
}
// This buffer is a disjoint tail of matrix scratch, live only while scanning.
// Reset for each fixed-x cross-check; tags cannot outlive its image or radius.
struct AxisPacked<'a> {
    bytes: &'a mut [u8],
}
#[derive(Clone, Copy)]
struct AxisSummary {
    sum: u16,
    min: u8,
    max: u8,
}
impl<'a> AxisPacked<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        for row in bytes.chunks_exact_mut(6) {
            row[..2].fill(0);
        }
        Self { bytes }
    }
    #[inline]
    fn row<const OPT: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<AxisSummary, Error> {
        let start = (y % 64) * 6;
        let entry = &self.bytes[start..start + 6];
        let key = (y + 1) as u16;
        if u16::from_le_bytes([entry[0], entry[1]]) == key {
            return Ok(AxisSummary {
                sum: u16::from_le_bytes([entry[2], entry[3]]),
                min: entry[4],
                max: entry[5],
            });
        }
        let mut sum = 0u16;
        let mut min = 255u8;
        let mut max = 0u8;
        for i in -2i32..=2 {
            let px = (x as i32 + i * bin.radius as i32 / 2).clamp(0, bin.image.width as i32 - 1)
                as usize;
            let value = bin.budget.read(bin.image, px, y)?;
            sum += value as u16;
            min = min.min(value);
            max = max.max(value);
        }
        let entry = &mut self.bytes[start..start + 6];
        entry[2..4].copy_from_slice(&sum.to_le_bytes());
        entry[4] = min;
        entry[5] = max;
        entry[..2].copy_from_slice(&key.to_le_bytes());
        Ok(AxisSummary { sum, min, max })
    }
    #[inline]
    fn dark<const OPT: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<bool, Error> {
        if let Some(dark) = bin.memo_get(x, y) {
            return Ok(dark ^ bin.inverted);
        }
        let pixel = bin.budget.read(bin.image, x, y)? as u32;
        let dark = 'threshold: {
            let mut sum = 0u32;
            let mut min = 255u8;
            let mut max = 0u8;
            for j in -2i32..=2 {
                let py = (y as i32 + j * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let row = self.row(bin, x, py)?;
                sum += row.sum as u32;
                min = min.min(row.min);
                max = max.max(row.max);
                if max - min >= 12 {
                    let remaining = (2 - j) as u32 * 5;
                    let target = pixel * 25;
                    if sum + remaining * bin.range[0] as u32 > target {
                        break 'threshold true;
                    }
                    if sum + remaining * bin.range[1] as u32 <= target {
                        break 'threshold false;
                    }
                }
            }
            if max - min < 12 {
                pixel <= bin.threshold as u32
            } else {
                pixel * 25 < sum
            }
        };
        bin.memo_set(x, y, dark);
        Ok(dark ^ bin.inverted)
    }
}
#[inline(never)]
fn cross_check_vertical_packed<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    expected: f32,
    bytes: &mut [u8],
) -> Result<Option<(f32, f32)>, Error> {
    let mut rows = AxisPacked::new(bytes);
    if bin.memo.is_some() {
        return cross_check_pixels(bin.image.height, y, expected, |p| rows.dark(bin, x, p));
    }
    // A completed classification at a run boundary can be reused when no
    // image-wide memo is present. Errors never populate this one-point cache.
    let mut last_p = usize::MAX;
    let mut last_dark = false;
    cross_check_pixels(bin.image.height, y, expected, |p| {
        if p == last_p {
            return Ok(last_dark);
        }
        let dark = rows.dark(bin, x, p)?;
        last_p = p;
        last_dark = dark;
        Ok(dark)
    })
}

fn cross_check<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    horizontal: bool,
    expected: f32,
) -> Result<Option<(f32, f32)>, Error> {
    let limit = if horizontal {
        bin.image.width
    } else {
        bin.image.height
    };
    let center = if horizontal { x } else { y };
    let get = |p: usize| {
        if horizontal {
            bin.dark(p, y)
        } else {
            bin.dark(x, p)
        }
    };
    cross_check_pixels(limit, center, expected, get)
}
// Global scans never need the local threshold or row-cache branches. Select
// their pixel accessor once per cross-check, preserving read order and budgets.
fn cross_check_global<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    horizontal: bool,
    expected: f32,
) -> Result<Option<(f32, f32)>, Error> {
    let limit = if horizontal {
        bin.image.width
    } else {
        bin.image.height
    };
    let center = if horizontal { x } else { y };
    let get = |p: usize| {
        let pixel = if horizontal {
            bin.budget.read(bin.image, p, y)?
        } else {
            bin.budget.read(bin.image, x, p)?
        };
        Ok((pixel <= bin.threshold) ^ bin.inverted)
    };
    cross_check_pixels(limit, center, expected, get)
}
fn cross_check_pixels(
    limit: usize,
    center: usize,
    expected: f32,
    mut get: impl FnMut(usize) -> Result<bool, Error>,
) -> Result<Option<(f32, f32)>, Error> {
    if !get(center)? {
        return Ok(None);
    }
    let mut r = [0usize; 5];
    let bound = (expected * 6.0 + 5.0) as usize;
    let mut low = center as isize;
    while low >= 0 && get(low as usize)? && r[2] <= bound {
        r[2] += 1;
        low -= 1;
    }
    while low >= 0 && !get(low as usize)? && r[1] <= bound {
        r[1] += 1;
        low -= 1;
    }
    while low >= 0 && get(low as usize)? && r[0] <= bound {
        r[0] += 1;
        low -= 1;
    }
    let mut high = center + 1;
    while high < limit && get(high)? && r[2] <= bound {
        r[2] += 1;
        high += 1;
    }
    let center_high = high;
    while high < limit && !get(high)? && r[3] <= bound {
        r[3] += 1;
        high += 1;
    }
    while high < limit && get(high)? && r[4] <= bound {
        r[4] += 1;
        high += 1;
    }
    let Some(module) = ratio(&r) else {
        return Ok(None);
    };
    if module < expected * 0.4 || module > expected * 2.5 {
        return Ok(None);
    }
    Ok(Some((center_high as f32 - r[2] as f32 * 0.5, module)))
}
fn add_finder(storage: &mut [u8], count: &mut usize, cap: usize, f: Finder) -> bool {
    for i in 0..*count {
        let old = read_finder(storage, i);
        if old.p.sub(f.p).norm() < old.pitch_x.max(old.pitch_y).max(f.pitch_x.max(f.pitch_y)) * 2.0
            && (old.pitch_x - f.pitch_x).abs() < old.pitch_x.max(f.pitch_x)
            && (old.pitch_y - f.pitch_y).abs() < old.pitch_y.max(f.pitch_y)
        {
            let weight = old.hits as f32;
            let divisor = weight + 1.0;
            write_finder(
                storage,
                i,
                Finder {
                    p: old.p.scale(weight).add(f.p).scale(1.0 / divisor),
                    pitch_x: (old.pitch_x * weight + f.pitch_x) / divisor,
                    pitch_y: (old.pitch_y * weight + f.pitch_y) / divisor,
                    hits: old.hits + 1,
                },
            );
            return true;
        }
    }
    if *count >= cap {
        let mut weakest = 0;
        for i in 1..cap {
            if read_finder(storage, i).hits < read_finder(storage, weakest).hits {
                weakest = i;
            }
        }
        if read_finder(storage, weakest).hits <= 1 {
            write_finder(storage, weakest, f);
        }
        return false;
    }
    write_finder(storage, *count, f);
    *count += 1;
    true
}
fn scan<const OPT: bool>(
    bin: Binary<'_, OPT>,
    storage: &mut [u8],
    cap: usize,
    step: usize,
    stats: &mut Stats,
    columns: Option<&mut [u8]>,
) -> Result<(usize, bool), Error> {
    if OPT && !bin.local {
        // Keep thresholding and the sequential read counter in the scan loop;
        // only cross-checks need the shared counter and general pixel accessor.
        return scan_global_register(bin, storage, cap, step, stats);
    }
    // Preserve these five rows during irregular cross-check/geometry access.
    // Only the continuous horizontal scan prefetches; other misses read directly.
    let scan_bin = Binary {
        fill_cache: true,
        ..bin
    };
    let (mut columns, mut axis) = if OPT {
        columns.map_or((None, None), |bytes| {
            let (columns, tail) = bytes.split_at_mut(bin.image.width * 4);
            (Some(ColumnSummary::new(columns)), tail.get_mut(..384))
        })
    } else {
        (None, None)
    };
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        if let Some(columns) = columns.as_mut() {
            columns.fill(bin, y)?;
        }
        let mut runs = [0usize; 5];
        let mut color = false;
        for x in 0..=bin.image.width {
            let next = x < bin.image.width
                && match columns.as_ref() {
                    Some(columns) => columns.dark(bin, x, y)?,
                    None => scan_bin.dark(x, y)?,
                };
            if next == color {
                runs[4] += 1;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        let vertical = match axis.as_deref_mut() {
                            Some(bytes) => {
                                cross_check_vertical_packed(bin, cx as usize, y, module, bytes)
                            }
                            None => cross_check(bin, cx as usize, y, false, module),
                        };
                        if let Some((cy, my)) = vertical? {
                            if let Some((rx, mx)) =
                                cross_check(bin, cx as usize, cy as usize, true, my)?
                            {
                                stats.finder_observations += 1;
                                let f = Finder {
                                    p: Point { x: rx, y: cy },
                                    pitch_x: mx,
                                    pitch_y: my,
                                    hits: 1,
                                };
                                if observation_pattern(bin, f)?
                                    && !add_finder(storage, &mut count, cap, f)
                                {
                                    overflow = true;
                                }
                            }
                        }
                    }
                }
            }
            runs.rotate_left(1);
            runs[4] = 1;
            color = next;
        }
    }
    // Stable insertion sort: strongest repeated observation first; ties keep scan order.
    for i in 1..count {
        let value = read_finder(storage, i);
        let mut j = i;
        while j > 0 {
            let prev = read_finder(storage, j - 1);
            if prev.hits >= value.hits {
                break;
            }
            write_finder(storage, j, prev);
            j -= 1;
        }
        write_finder(storage, j, value);
    }
    Ok((count, overflow))
}
// Compare eight byte lanes without borrowing between lanes, then gather MSBs.
#[inline]
#[cfg(target_pointer_width = "64")]
fn global_mask8(pixels: &[u8], threshold: u8) -> u8 {
    let word = u64::from_le_bytes(pixels[..8].try_into().unwrap());
    let high = 0x8080_8080_8080_8080u64;
    let low = 0x7f7f_7f7f_7f7f_7f7fu64;
    let threshold_low = u64::from(threshold & 0x7f) * 0x0101_0101_0101_0101;
    let compare = (threshold_low | high) - (word & low);
    let mask = if threshold < 128 {
        compare & !word
    } else {
        compare | !word
    } & high;
    (mask.wrapping_mul(0x0002_0408_1020_4081) >> 56) as u8
}
fn scan_global_register<const OPT: bool>(
    bin: Binary<'_, OPT>,
    storage: &mut [u8],
    cap: usize,
    step: usize,
    stats: &mut Stats,
) -> Result<(usize, bool), Error> {
    // At most one helper per buffered pixel. Each pair of monotonic axis
    // checks reads at most width + height + 14 pixels; four observation
    // patterns add at most 196. A full batch and all possible helpers must
    // fit before any read-ahead, so budget failures retain scalar positions.
    // Valid detector dimensions are <=32768; this product fits usize on 32-bit targets.
    let batch_guard = 16 * (bin.image.width + bin.image.height + 211);
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        let mut used = bin.budget.used.get();
        let mut runs = [0usize; 5];
        let mut color = false;
        let mut x = 0;
        let mut bits = 0u16;
        let mut buffered = 0usize;
        while x <= bin.image.width {
            let next = if x < bin.image.width {
                if buffered == 0
                    && bin.image.width - x >= 16
                    && bin.budget.limit - used >= batch_guard
                {
                    let start = y * bin.image.stride + x;
                    let pixels = &bin.image.data[start..start + 16];
                    #[cfg(target_pointer_width = "64")]
                    {
                        bits = u16::from(global_mask8(pixels, bin.threshold))
                            | (u16::from(global_mask8(&pixels[8..], bin.threshold)) << 8);
                        if bin.inverted {
                            bits = !bits;
                        }
                    }
                    #[cfg(not(target_pointer_width = "64"))]
                    {
                        bits = 0;
                        for (i, &pixel) in pixels.iter().enumerate() {
                            bits |= u16::from((pixel <= bin.threshold) ^ bin.inverted) << i;
                        }
                    }
                    used += 16;
                    buffered = 16;
                }
                if buffered != 0 {
                    let value = bits & 1 != 0;
                    bits >>= 1;
                    buffered -= 1;
                    value
                } else {
                    if used >= bin.budget.limit {
                        bin.budget.used.set(used);
                        return Err(Error::PixelBudget);
                    }
                    let pixel = bin.image.at(x, y);
                    used += 1;
                    (pixel <= bin.threshold) ^ bin.inverted
                }
            } else {
                false
            };
            if next == color {
                let same = if color {
                    (!bits).trailing_zeros()
                } else {
                    bits.trailing_zeros()
                } as usize;
                let skip = same.min(buffered);
                runs[4] += 1 + skip;
                bits >>= skip;
                buffered -= skip;
                x += 1 + skip;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    // Helpers share the counter. Commit sequential scan reads
                    // immediately before entering them, then reload afterward.
                    bin.budget.used.set(used);
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        if let Some((cy, my)) =
                            cross_check_global(bin, cx as usize, y, false, module)?
                        {
                            if let Some((rx, mx)) =
                                cross_check_global(bin, cx as usize, cy as usize, true, my)?
                            {
                                stats.finder_observations += 1;
                                let f = Finder {
                                    p: Point { x: rx, y: cy },
                                    pitch_x: mx,
                                    pitch_y: my,
                                    hits: 1,
                                };
                                if observation_pattern_global(bin, f)?
                                    && !add_finder(storage, &mut count, cap, f)
                                {
                                    overflow = true;
                                }
                            }
                        }
                    }
                    used = bin.budget.used.get();
                }
            }
            runs.rotate_left(1);
            runs[4] = 1;
            color = next;
            x += 1;
        }
        bin.budget.used.set(used);
    }
    // Stable insertion sort: strongest repeated observation first; ties keep scan order.
    for i in 1..count {
        let value = read_finder(storage, i);
        let mut j = i;
        while j > 0 {
            let prev = read_finder(storage, j - 1);
            if prev.hits >= value.hits {
                break;
            }
            write_finder(storage, j, prev);
            j -= 1;
        }
        write_finder(storage, j, value);
    }
    Ok((count, overflow))
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

/// Detects and decodes one Micro QR symbol using its single finder pattern.
///
/// This opt-in API shares [`Config`] validation, caller-owned scratch, pass
/// ordering, polarity handling, and work budgets with Model 2 detection. Supply
/// at least [`crate::core::MICRO_SCRATCH_BYTES`] bytes of core scratch.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
pub fn decode_micro<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<MicroDetection, Error> {
    decode_micro_with_stats(
        image,
        config,
        scratch,
        core_scratch,
        output,
        segments,
        &mut Stats::default(),
    )
}

/// Like [`decode_micro`], retaining work counters on success and failure.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
pub fn decode_micro_with_stats<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
) -> Result<MicroDetection, Error> {
    decode_micro_with_sampling(
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

/// Like [`decode_micro_with_stats`], with an explicit sampling policy.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
pub fn decode_micro_with_sampling<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
) -> Result<MicroDetection, Error> {
    *stats = Stats::default();
    validate_input(
        image,
        config,
        scratch,
        core_scratch,
        crate::core::MICRO_SCRATCH_BYTES,
    )?;
    let budget = PixelBudget::new(config.max_pixel_reads);
    let result = decode_micro_inner(
        image,
        config,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        &budget,
    );
    stats.pixel_reads = budget.used.get();
    result.map(|mut found| {
        found.stats = *stats;
        found
    })
}

#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
fn decode_micro_inner<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
) -> Result<MicroDetection, Error> {
    let (matrix, finders) = scratch.split_at_mut(MAX_MATRIX_BYTES);
    let mut min = 255u8;
    let mut max = 0u8;
    for y in 0..image.height {
        for x in 0..image.width {
            let value = budget.read(image, x, y)?;
            min = min.min(value);
            max = max.max(value);
        }
    }
    if max - min < 8 {
        return Err(Error::NotFound);
    }
    let threshold = ((min as u16 + max as u16) / 2) as u8;
    let mut overflow = false;
    let mut last_error = None;
    for pass in 0..4 {
        stats.passes += 1;
        let bin = Binary {
            image,
            budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold,
            inverted: pass % 2 != 0,
            local: pass >= 2,
            range: [min, max],
            radius: config.local_radius,
        };
        let (count, full) = scan(
            bin,
            finders,
            config.max_finders,
            config.row_step,
            stats,
            None,
        )?;
        overflow |= full;
        for index in 0..count {
            let finder = read_finder(finders, index);
            let mut search = MicroSearch {
                matrix,
                core_scratch,
                output,
                segments,
                stats,
                max_candidates: config.max_candidates,
                max_retries: config.max_retries,
                strategy,
                last_error: &mut last_error,
            };
            if let Some(found) = search.finder(bin, finder)? {
                return Ok(found);
            }
        }
    }
    if overflow {
        Err(Error::FinderCapacity)
    } else if let Some(error) = last_error {
        Err(Error::Decode(error))
    } else {
        Err(Error::NotFound)
    }
}

#[cfg(feature = "micro-qr-detect")]
struct MicroSearch<'a> {
    matrix: &'a mut [u8],
    core_scratch: &'a mut [u8],
    output: &'a mut [u8],
    segments: &'a mut [Segment],
    stats: &'a mut Stats,
    max_candidates: usize,
    max_retries: usize,
    strategy: SamplingStrategy,
    last_error: &'a mut Option<crate::core::Error>,
}

#[cfg(feature = "micro-qr-detect")]
impl MicroSearch<'_> {
    fn finder(&mut self, bin: Binary<'_>, finder: Finder) -> Result<Option<MicroDetection>, Error> {
        // A square finder has no orientation marker. Evaluate the same bounded
        // angle set used to validate finder observations, then all four inward
        // quadrant choices. Horizontal and vertical scan pitches retain
        // anisotropic scale information.
        for (cos, sin) in [
            (1.0f32, 0.0f32),
            (0.9238795, 0.38268343),
            (0.70710677, 0.70710677),
            (0.38268343, 0.9238795),
        ] {
            let chord_rotation = cos.max(sin);
            let u = Point {
                x: cos * finder.pitch_x * chord_rotation,
                y: sin * finder.pitch_y * chord_rotation,
            };
            let v = Point {
                x: -sin * finder.pitch_x * chord_rotation,
                y: cos * finder.pitch_y * chord_rotation,
            };
            for (u, v) in [
                (u, v),
                (v, u.scale(-1.0)),
                (u.scale(-1.0), v.scale(-1.0)),
                (v.scale(-1.0), u),
            ] {
                if self.stats.candidates >= self.max_candidates {
                    return Err(Error::CandidateBudget);
                }
                self.stats.candidates += 1;
                if !finder_pattern(bin, finder.p, u, v)? {
                    continue;
                }
                for side in [11usize, 13, 15, 17] {
                    let origin = finder.p.sub(u.scale(3.5)).sub(v.scale(3.5));
                    let h = Homography([u.x, v.x, origin.x, u.y, v.y, origin.y, 0.0, 0.0]);
                    if let Some(found) = self.attempt(bin, h, side)? {
                        return Ok(Some(found));
                    }
                }
            }
        }
        Ok(None)
    }

    fn attempt(
        &mut self,
        bin: Binary<'_>,
        h: Homography,
        side: usize,
    ) -> Result<Option<MicroDetection>, Error> {
        if !self.sample::<false>(bin, h, side)? {
            return Ok(None);
        }
        if let Some(found) = self.decode_sample(bin, h, side)? {
            return Ok(Some(found));
        }
        if self.strategy == SamplingStrategy::CenterThenMajority5 {
            if !self.sample::<true>(bin, h, side)? {
                return Ok(None);
            }
            return self.decode_sample(bin, h, side);
        }
        Ok(None)
    }

    fn sample<const MAJORITY: bool>(
        &mut self,
        bin: Binary<'_>,
        h: Homography,
        side: usize,
    ) -> Result<bool, Error> {
        if self.stats.retries >= self.max_retries {
            return Err(Error::RetryBudget);
        }
        self.stats.retries += 1;
        self.matrix[..(side * side).div_ceil(8)].fill(0);
        for y in 0..side {
            for x in 0..side {
                let mut votes = 0;
                let offsets = [
                    (0.0, 0.0),
                    (-0.25, 0.0),
                    (0.25, 0.0),
                    (0.0, -0.25),
                    (0.0, 0.25),
                ];
                for &(dx, dy) in &offsets[..if MAJORITY { 5 } else { 1 }] {
                    let Some(dark) = bin.point(h.at(Point {
                        x: x as f32 + 0.5 + dx,
                        y: y as f32 + 0.5 + dy,
                    }))?
                    else {
                        return Ok(false);
                    };
                    votes += usize::from(dark);
                }
                if votes >= if MAJORITY { 3 } else { 1 } {
                    let bit = y * side + x;
                    self.matrix[bit / 8] |= 0x80 >> (bit % 8);
                }
            }
        }
        Ok(true)
    }

    fn decode_sample(
        &mut self,
        bin: Binary<'_>,
        h: Homography,
        side: usize,
    ) -> Result<Option<MicroDetection>, Error> {
        let matrix = MicroMatrix::new(self.matrix, side).map_err(Error::Decode)?;
        match crate::core::decode_micro(matrix, self.core_scratch, self.output, self.segments) {
            Ok(decoded) => Ok(Some(MicroDetection {
                decoded,
                corners: projection_corners(h, side),
                inverted: bin.inverted,
                local_threshold: bin.local,
                stats: *self.stats,
            })),
            Err(
                error @ (crate::core::Error::OutputTooSmall
                | crate::core::Error::SegmentsTooSmall
                | crate::core::Error::ScratchTooSmall),
            ) => Err(Error::Decode(error)),
            Err(error) => {
                *self.last_error = Some(error);
                Ok(None)
            }
        }
    }
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
#[inline(never)]
fn complete_range(image: GrayImage<'_>) -> ([u8; 2], usize) {
    let mut min = 255u8;
    let mut max = 0u8;
    let mut rows = 0;
    for y in 0..image.height {
        let start = y * image.stride;
        for &value in &image.data[start..start + image.width] {
            min = min.min(value);
            max = max.max(value);
        }
        rows += 1;
        // No unread Gray8 value can extend the complete intensity range.
        // Check between rows so each contiguous reduction stays vectorizable.
        if min == 0 && max == 255 {
            break;
        }
    }
    ([min, max], rows * image.width)
}

// Scoped to one immutable ROI and one decode_region call only.
#[derive(Default)]
struct RegionState {
    range: Option<[u8; 2]>,
    memo_ready: bool,
}

#[allow(clippy::too_many_arguments)]
fn decode_region<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
    mut candidate_hint: Option<&mut Option<([Point; 4], usize)>>,
    cache: Option<&mut [u8]>,
    mode: DetectionMode,
) -> Result<Detection, Error> {
    let mut state = RegionState::default();
    // Dedicated row-cache APIs preserve their input-read order and cache-fill
    // semantics; public execution modes use the optimized uncached path below.
    if cache.is_some() {
        return decode_inner::<W, H, false>(
            image,
            config,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            budget,
            candidate_hint,
            cache,
            false,
            &mut state,
        );
    }
    let coarse = Config {
        row_step: if mode == DetectionMode::Fast {
            config.row_step.max(3)
        } else {
            config.row_step
        },
        ..*config
    };
    let passes_before = stats.passes;
    let result = decode_inner::<W, H, true>(
        image,
        &coarse,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        budget,
        candidate_hint.as_deref_mut(),
        None,
        mode == DetectionMode::Fast,
        &mut state,
    );
    match result {
        Err(error)
            if coarse.row_step != config.row_step
                && stats.passes > passes_before
                && recognition_failure(error) =>
        {
            decode_inner::<W, H, true>(
                image,
                config,
                strategy,
                scratch,
                core_scratch,
                output,
                segments,
                stats,
                budget,
                candidate_hint,
                None,
                mode == DetectionMode::Fast,
                &mut state,
            )
        }
        result => result,
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_inner<const W: usize, const H: usize, const OPT: bool>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
    candidate_hint: Option<&mut Option<([Point; 4], usize)>>,
    cache: Option<&mut [u8]>,
    fast: bool,
    state: &mut RegionState,
) -> Result<Detection, Error> {
    let cache = cache.map(|bytes| RefCell::new(RowCache::new(bytes)));
    let (matrix, remaining) = scratch.split_at_mut(MAX_MATRIX_BYTES);
    let (finders, tail) = remaining.split_at_mut(config.max_finders * FINDER_BYTES);
    let memo_bytes = (image.width * image.height).div_ceil(4);
    let memo = if fast && tail.len() >= memo_bytes {
        Some(Cell::from_mut(&mut tail[..memo_bytes]).as_slice_of_cells())
    } else {
        None
    };
    let [min, max] = if let Some(range) = state.range {
        range
    } else {
        let mut min = 255u8;
        let mut max = 0u8;
        let used = budget.used.get();
        let reads = image.width * image.height;
        if OPT && reads <= budget.limit - used {
            // No callbacks or fallible operations inside a complete ROI range scan.
            // Keep the partial-budget path scalar to preserve failure positions.
            let (range, actual_reads) = complete_range(image);
            [min, max] = range;
            budget.used.set(used + actual_reads);
        } else {
            for y in 0..image.height {
                for x in 0..image.width {
                    let value = budget.read(image, x, y)?;
                    min = min.min(value);
                    max = max.max(value);
                }
            }
        }
        let range = [min, max];
        state.range = Some(range);
        range
    };
    if max - min < 8 {
        return Err(Error::NotFound);
    }
    let threshold = ((min as u16 + max as u16) / 2) as u8;
    let mut overflow = false;
    let mut search = SearchContext::<OPT> {
        matrix,
        core_scratch,
        output,
        segments,
        stats,
        max_candidates: config.max_candidates,
        max_retries: config.max_retries,
        last_error: None,
        strategy,
        candidate_hint,
    };
    // Fast tries nearby global thresholds before the sampled local mean.
    // Keep the original midrange for local low-contrast fallback and memo reuse.
    let refine = OPT && fast && max - min >= 16;
    let local_start = if refine { 6 } else { 2 };
    for pass in 0..local_start + 2 {
        let threshold = match pass {
            2 | 3 if refine => threshold - (max - min) / 16,
            4 | 5 if refine => threshold + (max - min) / 16,
            _ => threshold,
        };
        if pass == local_start && !state.memo_ready {
            if let Some(memo) = memo {
                for byte in memo {
                    byte.set(0);
                }
                state.memo_ready = true;
            }
        }
        search.stats.passes += 1;
        let bin = Binary::<OPT> {
            image,
            budget,
            cache: cache.as_ref(),
            memo,
            fill_cache: false,
            threshold,
            inverted: pass % 2 != 0,
            local: pass >= local_start,
            range: [min, max],
            radius: config.local_radius,
        };
        let (count, full) = scan(
            bin,
            finders,
            config.max_finders,
            config.row_step,
            search.stats,
            if OPT && bin.local && image.width <= MAX_MATRIX_BYTES / 4 {
                Some(
                    &mut search.matrix[..if fast {
                        image.width * 4
                    } else {
                        MAX_MATRIX_BYTES
                    }],
                )
            } else {
                None
            },
        )?;
        overflow |= full;
        // Keep failed nearby-threshold probes from spending many sampling
        // attempts on projective sweeps. All attempts still share the config limit.
        let probe = refine && (2..6).contains(&pass);
        search.max_retries = if probe {
            config
                .max_retries
                .min(search.stats.retries.saturating_add(2))
        } else {
            config.max_retries
        };
        'candidates: for a in 0..count {
            for b in a + 1..count {
                for c in b + 1..count {
                    let set = [
                        read_finder(finders, a),
                        read_finder(finders, b),
                        read_finder(finders, c),
                    ];
                    match search.candidate(bin, set) {
                        Ok(Some(found)) => return Ok(found),
                        Ok(None) => {}
                        Err(Error::RetryBudget)
                            if probe && search.stats.retries < config.max_retries =>
                        {
                            break 'candidates
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
    if overflow {
        Err(Error::FinderCapacity)
    } else if let Some(e) = search.last_error {
        Err(Error::Decode(e))
    } else {
        Err(Error::NotFound)
    }
}
fn orient(f: [Finder; 3]) -> Option<(Finder, Finder, Finder)> {
    let distances = [
        f[1].p.sub(f[2].p).norm(),
        f[0].p.sub(f[2].p).norm(),
        f[0].p.sub(f[1].p).norm(),
    ];
    let mut index = 0;
    for i in 1..3 {
        if distances[i] > distances[index] {
            index = i;
        }
    }
    let tl = f[index];
    let mut tr = f[(index + 1) % 3];
    let mut bl = f[(index + 2) % 3];
    let a = tr.p.sub(tl.p);
    let b = bl.p.sub(tl.p);
    let na = a.norm();
    let nb = b.norm();
    if na < 10.0 || nb < 10.0 || na / nb < 0.4 || na / nb > 2.5 || a.dot(b).abs() > na * nb * 0.60 {
        return None;
    }
    let min_x = tl.pitch_x.min(tr.pitch_x.min(bl.pitch_x));
    let max_x = tl.pitch_x.max(tr.pitch_x.max(bl.pitch_x));
    let min_y = tl.pitch_y.min(tr.pitch_y.min(bl.pitch_y));
    let max_y = tl.pitch_y.max(tr.pitch_y.max(bl.pitch_y));
    if min_x < 0.7 || min_y < 0.7 || max_x > min_x * 3.0 || max_y > min_y * 3.0 {
        return None;
    }
    if a.cross(b) < 0.0 {
        core::mem::swap(&mut tr, &mut bl);
    }
    Some((tl, tr, bl))
}
fn find_alignment<const OPT: bool>(
    bin: Binary<'_, OPT>,
    prediction: Point,
    u: Point,
    v: Point,
) -> Result<Option<Point>, Error> {
    if OPT && !bin.local {
        return find_alignment_points::<OPT>(prediction, u, v, |p| bin.global_point(p));
    }
    find_alignment_points::<OPT>(prediction, u, v, |p| bin.point(p))
}
fn find_alignment_points<const OPT: bool>(
    prediction: Point,
    u: Point,
    v: Point,
    get: impl Fn(Point) -> Result<Option<bool>, Error>,
) -> Result<Option<Point>, Error> {
    let mut best = None;
    let mut best_score = -1.0f32;
    // Search up to eight modules around the affine prediction, half-module increments.
    for iy in -16..=16 {
        for ix in -16..=16 {
            let center = prediction
                .add(u.scale(ix as f32 * 0.5))
                .add(v.scale(iy as f32 * 0.5));
            let penalty = (ix * ix + iy * iy) as f32 * 0.003;
            if OPT && 25.0 - penalty <= best_score {
                continue;
            }
            let mut matches = 0;
            let mut valid = true;
            let mut examined = 0;
            'pattern: for y in -2i32..=2 {
                for x in -2i32..=2 {
                    let p = center.add(u.scale(x as f32)).add(v.scale(y as f32));
                    let expected = x.abs() == 2 || y.abs() == 2 || (x == 0 && y == 0);
                    if let Some(d) = get(p)? {
                        matches += usize::from(d == expected);
                    } else {
                        valid = false;
                    }
                    examined += 1;
                    // At least 23 of 25 points must match. Preserve traversal
                    // and strict score ordering, including ties, while dropping
                    // candidates that cannot become the selected result.
                    if OPT
                        && (!valid
                            || examined - matches > 2
                            || (matches + 25 - examined) as f32 - penalty <= best_score)
                    {
                        valid = false;
                        break 'pattern;
                    }
                }
            }
            if valid && matches >= 23 {
                let score = matches as f32 - penalty;
                if score > best_score {
                    best_score = score;
                    best = Some(center);
                }
            }
        }
    }
    Ok(best)
}
#[derive(Clone, Copy)]
struct Homography([f32; 8]);
impl Homography {
    fn at(self, p: Point) -> Point {
        let h = self.0;
        let d = h[6] * p.x + h[7] * p.y + 1.0;
        Point {
            x: (h[0] * p.x + h[1] * p.y + h[2]) / d,
            y: (h[3] * p.x + h[4] * p.y + h[5]) / d,
        }
    }
}
// Explicit matrix indices keep Gaussian elimination and pivoting auditable.
#[allow(clippy::needless_range_loop)]
fn homography(src: [Point; 4], dst: [Point; 4]) -> Option<Homography> {
    let mut a = [[0.0f32; 9]; 8];
    for i in 0..4 {
        let p = src[i];
        let q = dst[i];
        a[2 * i] = [p.x, p.y, 1.0, 0.0, 0.0, 0.0, -q.x * p.x, -q.x * p.y, q.x];
        a[2 * i + 1] = [0.0, 0.0, 0.0, p.x, p.y, 1.0, -q.y * p.x, -q.y * p.y, q.y];
    }
    for col in 0..8 {
        let mut pivot = col;
        for row in col + 1..8 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-6 {
            return None;
        }
        a.swap(pivot, col);
        let div = a[col][col];
        for j in col..9 {
            a[col][j] /= div;
        }
        for row in 0..8 {
            if row == col {
                continue;
            }
            let k = a[row][col];
            for j in col..9 {
                a[row][j] -= k * a[col][j];
            }
        }
    }
    let mut h = [0.0; 8];
    for i in 0..8 {
        h[i] = a[i][8];
    }
    Some(Homography(h))
}
/// Buffers and budgets shared by all threshold passes and projection attempts.
#[derive(Debug, PartialEq, Eq)]
enum SampleOutcome {
    Complete,
    InvalidGeometry,
    HeaderRejected,
}
struct SearchContext<'a, const OPT: bool = false> {
    matrix: &'a mut [u8],
    core_scratch: &'a mut [u8],
    output: &'a mut [u8],
    segments: &'a mut [Segment],
    stats: &'a mut Stats,
    max_candidates: usize,
    max_retries: usize,
    last_error: Option<crate::core::Error>,
    strategy: SamplingStrategy,
    candidate_hint: Option<&'a mut Option<([Point; 4], usize)>>,
}
impl<const OPT: bool> SearchContext<'_, OPT> {
    fn candidate(
        &mut self,
        bin: Binary<'_, OPT>,
        set: [Finder; 3],
    ) -> Result<Option<Detection>, Error> {
        if self.stats.candidates >= self.max_candidates {
            return Err(Error::CandidateBudget);
        }
        self.stats.candidates += 1;
        let Some((tl, tr, bl)) = orient(set) else {
            return Ok(None);
        };
        let ax = tr.p.sub(tl.p);
        let ay = bl.p.sub(tl.p);
        // Scan pitches are central chords along the image axes, not symbol axes.
        // Normalize each finder span by its horizontal/vertical chord first.
        // The dominant component corrects the chord length for symbol rotation.
        let module_span = |axis: Point, end: Finder| {
            let x = axis.x / ((tl.pitch_x + end.pitch_x) * 0.5);
            let y = axis.y / ((tl.pitch_y + end.pitch_y) * 0.5);
            (x * x + y * y) / x.abs().max(y.abs())
        };
        let modules = (module_span(ax, tr) + module_span(ay, bl)) * 0.5 + 7.0;
        let estimate = ((modules - 17.0) / 4.0 + 0.5) as i32;
        for delta in [0, -1, 1, -2, 2] {
            let version = estimate + delta;
            if !(1..=40).contains(&version) {
                continue;
            }
            if let Some(found) = self.version(bin, [tl, tr, bl], version as u8)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn version(
        &mut self,
        bin: Binary<'_, OPT>,
        finders: [Finder; 3],
        version: u8,
    ) -> Result<Option<Detection>, Error> {
        let [tl, tr, bl] = finders;
        let side = version as usize * 4 + 17;
        let u = tr.p.sub(tl.p).scale(1.0 / (side as f32 - 7.0));
        let v = bl.p.sub(tl.p).scale(1.0 / (side as f32 - 7.0));
        if !finder_pattern(bin, tl.p, u, v)?
            || !finder_pattern(bin, tr.p, u, v)?
            || !finder_pattern(bin, bl.p, u, v)?
        {
            return Ok(None);
        }
        let anchor = side as f32 - 6.5;
        let predicted = tl.p.add(u.add(v).scale(anchor - 3.5));
        let aligned = if version > 1 {
            find_alignment(bin, predicted, u, v)?
        } else {
            None
        };
        if let Some(p) = aligned {
            if let Some(h) = homography(
                [
                    Point { x: 3.5, y: 3.5 },
                    Point {
                        x: side as f32 - 3.5,
                        y: 3.5,
                    },
                    Point {
                        x: 3.5,
                        y: side as f32 - 3.5,
                    },
                    Point {
                        x: anchor,
                        y: anchor,
                    },
                ],
                [tl.p, tr.p, bl.p, p],
            ) {
                if let Some(found) = self.attempt(bin, h, side)? {
                    return Ok(Some(found));
                }
            }
        }
        // Affine first, then bounded projective fourth-point hypotheses, including V1.
        for (dx, dy) in [
            (0.0, 0.0),
            (-0.10, 0.0),
            (0.10, 0.0),
            (0.0, -0.10),
            (0.0, 0.10),
            (-0.10, -0.10),
            (0.10, 0.10),
            (-0.10, 0.10),
            (0.10, -0.10),
        ] {
            let fourth =
                tr.p.add(bl.p)
                    .sub(tl.p)
                    .add(tr.p.sub(tl.p).scale(dx))
                    .add(bl.p.sub(tl.p).scale(dy));
            if let Some(h) = homography(
                [
                    Point { x: 3.5, y: 3.5 },
                    Point {
                        x: side as f32 - 3.5,
                        y: 3.5,
                    },
                    Point {
                        x: 3.5,
                        y: side as f32 - 3.5,
                    },
                    Point {
                        x: side as f32 - 3.5,
                        y: side as f32 - 3.5,
                    },
                ],
                [tl.p, tr.p, bl.p, fourth],
            ) {
                if let Some(found) = self.attempt(bin, h, side)? {
                    return Ok(Some(found));
                }
            }
        }
        Ok(None)
    }

    fn attempt(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<Option<Detection>, Error> {
        match self.sample::<false>(bin, h, side)? {
            SampleOutcome::InvalidGeometry => return Ok(None),
            SampleOutcome::Complete => {
                if let Some(found) = self.decode_sample(bin, h, side)? {
                    return Ok(Some(found));
                }
            }
            SampleOutcome::HeaderRejected => {}
        }
        if self.strategy == SamplingStrategy::CenterThenMajority5 {
            return match self.sample::<true>(bin, h, side)? {
                SampleOutcome::Complete => self.decode_sample(bin, h, side),
                SampleOutcome::InvalidGeometry | SampleOutcome::HeaderRejected => Ok(None),
            };
        }
        Ok(None)
    }

    fn sample<const MAJORITY: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<SampleOutcome, Error> {
        if self.stats.retries >= self.max_retries {
            return Err(Error::RetryBudget);
        }
        self.stats.retries += 1;
        // Repeating the existing Format/Version error or rejecting geometry
        // leaves the final error/hint unchanged. Restrict
        // filtering to the last sample: a geometry failure of the initial
        // Center sample must never add a Majority retry.
        let reads_per_module = if MAJORITY { 5 } else { 1 };
        let header_modules = if side >= 45 { 67 } else { 31 };
        let reserved_reads = (side * side + header_modules) * reads_per_module;
        if OPT
            && !bin.local
            && (MAJORITY || self.strategy == SamplingStrategy::Center)
            && matches!(
                self.last_error,
                Some(crate::core::Error::Format | crate::core::Error::Version)
            )
            && reserved_reads <= bin.budget.limit - bin.budget.used.get()
        {
            if !self.sample_header::<MAJORITY>(bin, h, side)? {
                return Ok(SampleOutcome::InvalidGeometry);
            }
            let matrix = Matrix::new(self.matrix, side).map_err(Error::Decode)?;
            if crate::core::header_error(matrix).is_some_and(|error| Some(error) == self.last_error)
            {
                return Ok(SampleOutcome::HeaderRejected);
            }
        }
        let complete = if OPT && !bin.local {
            self.sample_pixels::<MAJORITY>(h, side, |p| bin.global_point(p))?
        } else {
            self.sample_pixels::<MAJORITY>(h, side, |p| bin.point(p))?
        };
        Ok(if complete {
            SampleOutcome::Complete
        } else {
            SampleOutcome::InvalidGeometry
        })
    }

    fn sample_header<const MAJORITY: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<bool, Error> {
        self.matrix[..(side * side).div_ceil(8)].fill(0);
        // Union of the two format copies in both orientations: 31 modules.
        for coordinate in (0..6).chain(7..9).chain(side - 8..side) {
            for (x, y) in [(8, coordinate), (coordinate, 8)] {
                if coordinate == 8 && x == coordinate && y == 8 {
                    // Both entries are (8,8); visit it once below.
                    continue;
                }
                if !self.sample_header_point::<MAJORITY>(bin, h, side, x, y)? {
                    return Ok(false);
                }
            }
        }
        if !self.sample_header_point::<MAJORITY>(bin, h, side, 8, 8)? {
            return Ok(false);
        }
        if side >= 45 {
            for i in 0..18 {
                let x = side - 11 + i % 3;
                let y = i / 3;
                if !self.sample_header_point::<MAJORITY>(bin, h, side, x, y)?
                    || !self.sample_header_point::<MAJORITY>(bin, h, side, y, x)?
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn sample_header_point<const MAJORITY: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
        x: usize,
        y: usize,
    ) -> Result<bool, Error> {
        let p = Point {
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
        };
        let offsets = [
            (0.0, 0.0),
            (-0.25, 0.0),
            (0.25, 0.0),
            (0.0, -0.25),
            (0.0, 0.25),
        ];
        let mut votes = 0;
        for &(dx, dy) in &offsets[..if MAJORITY { 5 } else { 1 }] {
            let Some(dark) = bin.global_point(h.at(Point {
                x: p.x + dx,
                y: p.y + dy,
            }))?
            else {
                return Ok(false);
            };
            votes += usize::from(dark);
        }
        if votes >= if MAJORITY { 3 } else { 1 } {
            let i = y * side + x;
            self.matrix[i / 8] |= 128 >> (i % 8);
        }
        Ok(true)
    }

    #[cfg(test)]
    fn attempt_reference(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<Option<Detection>, Error> {
        if !self.sample_reference::<false>(bin, h, side)? {
            return Ok(None);
        }
        if let Some(found) = self.decode_sample(bin, h, side)? {
            return Ok(Some(found));
        }
        if self.strategy == SamplingStrategy::CenterThenMajority5 {
            if !self.sample_reference::<true>(bin, h, side)? {
                return Ok(None);
            }
            return self.decode_sample(bin, h, side);
        }
        Ok(None)
    }

    #[cfg(test)]
    fn sample_reference<const MAJORITY: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<bool, Error> {
        if self.stats.retries >= self.max_retries {
            return Err(Error::RetryBudget);
        }
        self.stats.retries += 1;
        if OPT && !bin.local {
            self.sample_pixels::<MAJORITY>(h, side, |p| bin.global_point(p))
        } else {
            self.sample_pixels::<MAJORITY>(h, side, |p| bin.point(p))
        }
    }

    fn sample_pixels<const MAJORITY: bool>(
        &mut self,
        h: Homography,
        side: usize,
        get: impl Fn(Point) -> Result<Option<bool>, Error>,
    ) -> Result<bool, Error> {
        self.matrix[..(side * side).div_ceil(8)].fill(0);
        let mut index = 0usize;
        let mut packed = 0u8;
        for y in 0..side {
            for x in 0..side {
                let p = Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                let mut votes = 0;
                let offsets = [
                    (0.0, 0.0),
                    (-0.25, 0.0),
                    (0.25, 0.0),
                    (0.0, -0.25),
                    (0.0, 0.25),
                ];
                for &(dx, dy) in &offsets[..if MAJORITY { 5 } else { 1 }] {
                    let dark = match get(h.at(Point {
                        x: p.x + dx,
                        y: p.y + dy,
                    })) {
                        Ok(Some(dark)) => dark,
                        Ok(None) => {
                            self.matrix[index / 8] = packed;
                            return Ok(false);
                        }
                        Err(error) => {
                            self.matrix[index / 8] = packed;
                            return Err(error);
                        }
                    };
                    votes += usize::from(dark);
                }
                if votes >= if MAJORITY { 3 } else { 1 } {
                    packed |= 128 >> (index % 8);
                }
                if index % 8 == 7 {
                    self.matrix[index / 8] = packed;
                    packed = 0;
                }
                index += 1;
            }
        }
        if index % 8 != 0 {
            self.matrix[index / 8] = packed;
        }
        Ok(true)
    }

    #[cfg(test)]
    fn sample_pixels_reference<const MAJORITY: bool>(
        &mut self,
        h: Homography,
        side: usize,
        get: impl Fn(Point) -> Result<Option<bool>, Error>,
    ) -> Result<bool, Error> {
        self.matrix[..(side * side).div_ceil(8)].fill(0);
        for y in 0..side {
            for x in 0..side {
                let p = Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                let mut votes = 0;
                let offsets = [
                    (0.0, 0.0),
                    (-0.25, 0.0),
                    (0.25, 0.0),
                    (0.0, -0.25),
                    (0.0, 0.25),
                ];
                for &(dx, dy) in &offsets[..if MAJORITY { 5 } else { 1 }] {
                    let Some(dark) = get(h.at(Point {
                        x: p.x + dx,
                        y: p.y + dy,
                    }))?
                    else {
                        return Ok(false);
                    };
                    votes += usize::from(dark);
                }
                if votes >= if MAJORITY { 3 } else { 1 } {
                    let i = y * side + x;
                    self.matrix[i / 8] |= 128 >> (i % 8);
                }
            }
        }
        Ok(true)
    }
    fn decode_sample(
        &mut self,
        bin: Binary<'_, OPT>,
        h: Homography,
        side: usize,
    ) -> Result<Option<Detection>, Error> {
        let corners = projection_corners(h, side);
        let m = Matrix::new(self.matrix, side).map_err(Error::Decode)?;
        match crate::core::decode(m, self.core_scratch, self.output, self.segments) {
            Ok(decoded) => Ok(Some(Detection {
                decoded,
                corners,
                inverted: bin.inverted,
                local_threshold: bin.local,
                stats: *self.stats,
            })),
            Err(
                e @ (crate::core::Error::OutputTooSmall
                | crate::core::Error::SegmentsTooSmall
                | crate::core::Error::ScratchTooSmall),
            ) => Err(Error::Decode(e)),
            Err(e) => {
                if candidate_content_error(e) {
                    if let Some(hint) = self.candidate_hint.as_deref_mut() {
                        if hint.is_none() && track_hint(corners, side, bin.image).is_some() {
                            *hint = Some((corners, side));
                        }
                    }
                }
                self.last_error = Some(e);
                Ok(None)
            }
        }
    }
}

fn projection_corners(h: Homography, side: usize) -> [Point; 4] {
    let n = side as f32;
    [
        h.at(Point { x: 0.0, y: 0.0 }),
        h.at(Point { x: n, y: 0.0 }),
        h.at(Point { x: n, y: n }),
        h.at(Point { x: 0.0, y: n }),
    ]
}
fn candidate_content_error(e: crate::core::Error) -> bool {
    matches!(
        e,
        crate::core::Error::ReedSolomon
            | crate::core::Error::Truncated
            | crate::core::Error::MalformedPayload
            | crate::core::Error::UnsupportedMode
    )
}
fn recognition_failure(e: Error) -> bool {
    matches!(
        e,
        Error::NotFound
            | Error::Decode(
                crate::core::Error::Format
                    | crate::core::Error::Version
                    | crate::core::Error::ReedSolomon
                    | crate::core::Error::Truncated
                    | crate::core::Error::MalformedPayload
                    | crate::core::Error::UnsupportedMode
            )
    )
}

/// A search hint, not proof that a QR symbol exists. It retains no image or payload.
/// Coordinates refer to the complete image passed to [`Tracker::decode_with_stats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackHint {
    /// Expanded, clamped search rectangle in input-image coordinates.
    pub roi: Rect,
    /// Width of the input image from which this hint was obtained.
    pub image_width: usize,
    /// Height of the input image from which this hint was obtained.
    pub image_height: usize,
}
/// Frame-to-frame search state. Candidates expire after two complete frame misses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackerState {
    /// Search the whole input image.
    #[default]
    Searching,
    /// An unconfirmed geometry or previous detection survived one frame miss.
    Candidate {
        /// Search rectangle; it does not confirm QR presence.
        hint: TrackHint,
        /// Consecutive completely searched frames without a decoded symbol.
        misses: u8,
    },
    /// The previous frame successfully decoded at this location.
    Tracking {
        /// Expanded location of the successful symbol.
        hint: TrackHint,
    },
}
/// Frame counters; ROI and full-image searches share one instance and all budgets.
pub type TrackerStats = Stats;

/// Caller-owned, allocation-free tracker containing only geometry and state.
///
/// A saved ROI is searched first, followed by the full image in the same frame
/// after ordinary recognition failure. A full-image ROI is never searched twice.
/// Budget, validation, and capacity failures leave the previous state unchanged.
/// Different image dimensions invalidate the saved hint for that call. For a new
/// crop origin or scale with unchanged dimensions, the caller must call [`Self::reset`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    state: TrackerState,
}
impl Tracker {
    /// Creates a tracker in [`TrackerState::Searching`].
    pub const fn new() -> Self {
        Self {
            state: TrackerState::Searching,
        }
    }
    /// Restores a caller-saved state after validating its integer geometry.
    ///
    /// Candidate misses must be exactly one. Saved image dimensions must be
    /// nonzero and at most 32768, with a nonempty rectangle entirely inside
    /// those dimensions. Validation uses saved dimensions, so a later input
    /// resize still follows the ordinary full-image search rule.
    pub fn from_state(state: TrackerState) -> Result<Self, Error> {
        let hint = match state {
            TrackerState::Searching => return Ok(Self::new()),
            TrackerState::Candidate { hint, misses: 1 } | TrackerState::Tracking { hint } => hint,
            TrackerState::Candidate { .. } => return Err(Error::InvalidConfig),
        };
        if hint.image_width == 0
            || hint.image_height == 0
            || hint.image_width > 32768
            || hint.image_height > 32768
            || hint.roi.width == 0
            || hint.roi.height == 0
            || hint
                .roi
                .x
                .checked_add(hint.roi.width)
                .is_none_or(|right| right > hint.image_width)
            || hint
                .roi
                .y
                .checked_add(hint.roi.height)
                .is_none_or(|bottom| bottom > hint.image_height)
        {
            return Err(Error::InvalidImage);
        }
        Ok(Self { state })
    }
    /// Discards previous geometry and starts a full-image search next frame.
    pub fn reset(&mut self) {
        self.state = TrackerState::Searching;
    }
    /// Returns the current state and integer search geometry.
    pub const fn state(&self) -> TrackerState {
        self.state
    }

    /// Decodes one frame in Direct mode with caller-owned buffers and
    /// cumulative frame counters.
    ///
    /// Bounds and buffer requirements are validated against the complete input
    /// before selecting a ROI. Returned corners use input-image coordinates.
    /// A successful decode enters `Tracking`; a first complete miss enters
    /// `Candidate` when a hint exists; a second enters `Searching`. A new failed
    /// candidate cannot reset the miss count. Only a complete recognition failure
    /// or success commits state. Failure buffer contents are unspecified.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_stats<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
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
    /// Like [`Self::decode_with_stats`], with an explicit execution mode.
    ///
    /// Each ROI/full-image search applies the selected mode. Fast's dense
    /// fallback, ROI recovery and full-image search all share the frame budgets.
    /// The scratch requirements and state-transition rules are unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_mode<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        mode: DetectionMode,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
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

    /// Row-cache variant of [`Self::decode_with_stats`]. Uses five full-image-width
    /// rows of internal RAM. ROI and full-frame retries reset tags and share budgets.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_stats_cached<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
        cache: &mut [u8],
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
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
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
        mut cache: Option<&mut [u8]>,
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
        let previous = match self.state {
            TrackerState::Candidate { hint, .. } | TrackerState::Tracking { hint }
                if hint.image_width != image.width || hint.image_height != image.height =>
            {
                TrackerState::Searching
            }
            state => state,
        };
        let saved = match previous {
            TrackerState::Searching => None,
            TrackerState::Candidate { hint, .. } | TrackerState::Tracking { hint } => Some(hint),
        };
        let full = Rect {
            x: 0,
            y: 0,
            width: image.width,
            height: image.height,
        };
        let budget = PixelBudget::new(config.max_pixel_reads);
        let mut candidate = None;
        // This closure reborrows every buffer but retains one frame's counters/budget.
        let mut run = |rect: Rect| {
            let region = image.sub_roi(rect)?;
            let mut geometry = None;
            let result = decode_region(
                region,
                config,
                strategy,
                scratch,
                core_scratch,
                output,
                segments,
                stats,
                &budget,
                Some(&mut geometry),
                cache.as_deref_mut(),
                mode,
            );
            let translate = |mut corners: [Point; 4]| {
                for p in &mut corners {
                    p.x += rect.x as f32;
                    p.y += rect.y as f32;
                }
                corners
            };
            if candidate.is_none() {
                if let Some((corners, side)) = geometry {
                    candidate = track_hint(translate(corners), side, image);
                }
            }
            result.map(|mut found| {
                found.corners = translate(found.corners);
                found
            })
        };
        let result = if let Some(hint) = saved.filter(|h| h.roi != full) {
            match run(hint.roi) {
                Err(e) if recognition_failure(e) => run(full),
                result => result,
            }
        } else {
            run(full)
        };
        stats.pixel_reads = budget.used.get();
        match result {
            Ok(mut found) => {
                found.stats = *stats;
                self.state = match track_hint(
                    found.corners,
                    found.decoded.version as usize * 4 + 17,
                    image,
                ) {
                    Some(hint) => TrackerState::Tracking { hint },
                    None => TrackerState::Searching,
                };
                Ok(found)
            }
            Err(e) if recognition_failure(e) => {
                self.state = match previous {
                    TrackerState::Candidate { .. } => TrackerState::Searching,
                    _ => match candidate.or(saved) {
                        Some(hint) => TrackerState::Candidate { hint, misses: 1 },
                        None => TrackerState::Searching,
                    },
                };
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}
fn track_hint(corners: [Point; 4], side: usize, image: GrayImage<'_>) -> Option<TrackHint> {
    if corners.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let mut min = corners[0];
    let mut max = corners[0];
    let mut edge = 0.0f32;
    let mut area = 0.0f32;
    for i in 0..4 {
        min.x = min.x.min(corners[i].x);
        min.y = min.y.min(corners[i].y);
        max.x = max.x.max(corners[i].x);
        max.y = max.y.max(corners[i].y);
        let next = corners[(i + 1) % 4];
        edge = edge.max(next.sub(corners[i]).norm());
        area += corners[i].cross(next);
    }
    if !edge.is_finite()
        || !area.is_finite()
        || area.abs() < 1.0
        || max.x <= min.x
        || max.y <= min.y
        || side == 0
    {
        return None;
    }
    let margin = (edge * 0.25).max(edge * 4.0 / side as f32).max(8.0);
    let x = (min.x - margin).clamp(0.0, image.width as f32) as usize;
    let y = (min.y - margin).clamp(0.0, image.height as f32) as usize;
    let ceil = |n: f32| {
        let i = n as usize;
        i + usize::from(n > i as f32)
    };
    let right = ceil((max.x + margin).clamp(0.0, image.width as f32));
    let bottom = ceil((max.y + margin).clamp(0.0, image.height as f32));
    if right <= x || bottom <= y {
        return None;
    }
    Some(TrackHint {
        roi: Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        },
        image_width: image.width,
        image_height: image.height,
    })
}

fn finder_pattern<const OPT: bool>(
    bin: Binary<'_, OPT>,
    center: Point,
    u: Point,
    v: Point,
) -> Result<bool, Error> {
    let mut good = 0;
    for y in -3i32..=3 {
        for x in -3i32..=3 {
            let edge = x.abs().max(y.abs());
            let expected = edge == 3 || edge <= 1;
            let p = center.add(u.scale(x as f32)).add(v.scale(y as f32));
            if bin.point(p)? == Some(expected) {
                good += 1;
            }
        }
    }
    Ok(good >= 41)
}

fn observation_pattern<const OPT: bool>(bin: Binary<'_, OPT>, f: Finder) -> Result<bool, Error> {
    observation_pattern_pixels(f, |p| bin.point(p))
}
fn observation_pattern_global<const OPT: bool>(
    bin: Binary<'_, OPT>,
    f: Finder,
) -> Result<bool, Error> {
    observation_pattern_pixels(f, |p| bin.global_point(p))
}

fn observation_pattern_pixels(
    f: Finder,
    get: impl Fn(Point) -> Result<Option<bool>, Error>,
) -> Result<bool, Error> {
    for (cos, sin) in [
        (1.0f32, 0.0f32),
        (0.9238795, 0.38268343),
        (0.70710677, 0.70710677),
        (0.38268343, 0.9238795),
    ] {
        let chord_rotation = cos.max(sin);
        let u = Point {
            x: cos * f.pitch_x * chord_rotation,
            y: sin * f.pitch_y * chord_rotation,
        };
        let v = Point {
            x: -sin * f.pitch_x * chord_rotation,
            y: cos * f.pitch_y * chord_rotation,
        };
        let mut good = 0;
        for y in -3i32..=3 {
            for x in -3i32..=3 {
                let edge = x.abs().max(y.abs());
                let expected = edge == 3 || edge <= 1;
                let p = f.p.add(u.scale(x as f32)).add(v.scale(y as f32));
                if get(p)? == Some(expected) {
                    good += 1;
                }
            }
        }
        if good >= 37 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod tests {
    #[test]
    fn batched_global_rows_differential_boundaries_long_runs_and_helpers() {
        let mut state = 0x811ab32du32;
        let mut cases = 0;
        let mut observations = 0;
        for width in [1usize, 7, 8, 9, 15, 16, 17, 31, 32, 33, 97, 257] {
            let height = 35;
            let stride = width + 5;
            for pattern in 0..3 {
                let mut data: std::vec::Vec<_> = (0..height * stride)
                    .map(|i| {
                        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                        match pattern {
                            0 => (state >> 24) as u8,
                            1 if (i % stride / 13) % 2 == 0 => 0,
                            _ => 255,
                        }
                    })
                    .collect();
                if pattern == 2 && width >= 33 {
                    for my in 0..7 {
                        for mx in 0..7 {
                            let dark = mx == 0
                                || my == 0
                                || mx == 6
                                || my == 6
                                || ((2..=4).contains(&mx) && (2..=4).contains(&my));
                            for yy in 0..3 {
                                for xx in 0..3 {
                                    data[(4 + my * 3 + yy) * stride + 4 + mx * 3 + xx] =
                                        if dark { 0 } else { 255 };
                                }
                            }
                        }
                    }
                }
                let image = GrayImage::new(&data, width, height, stride).unwrap();
                for inverted in [false, true] {
                    for threshold in [0, 127, 255] {
                        for step in [1, 2, 3] {
                            for limit in [
                                0,
                                1,
                                2,
                                6,
                                7,
                                8,
                                9,
                                14,
                                15,
                                16,
                                17,
                                31,
                                32,
                                33,
                                255,
                                999,
                                width * height - 1,
                                width * height,
                                width * height + 1,
                                16 * (width + height + 211) - 1,
                                16 * (width + height + 211),
                                16 * (width + height + 211) + 1,
                                usize::MAX,
                            ] {
                                let rb = PixelBudget::new(limit);
                                let ab = PixelBudget::new(limit);
                                let reference = Binary {
                                    threshold,
                                    inverted,
                                    ..test_binary(image, &rb, false)
                                };
                                let actual = Binary::<true> {
                                    image,
                                    budget: &ab,
                                    cache: None,
                                    memo: None,
                                    fill_cache: false,
                                    threshold,
                                    inverted,
                                    local: false,
                                    range: [0, 255],
                                    radius: 24,
                                };
                                let mut rs = [0xaau8; 32 * FINDER_BYTES + 7];
                                let mut actual_storage = rs;
                                let mut rst = Stats::default();
                                let mut ast = Stats::default();
                                let r = scan(reference, &mut rs, 32, step, &mut rst, None);
                                let a = scan(actual, &mut actual_storage, 32, step, &mut ast, None);
                                assert_eq!(r,a,"width={width} pattern={pattern} threshold={threshold} step={step} limit={limit}");
                                assert_eq!(rb.used.get(), ab.used.get());
                                assert_eq!(rs, actual_storage);
                                assert_eq!(rst, ast);
                                observations += ast.finder_observations;
                                cases += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(observations > 0);
        std::println!("differential cases={cases}, helper observations={observations}");
    }

    #[test]
    fn batched_global_rows_never_exceed_partial_budget() {
        let data: std::vec::Vec<_> = (0..35 * 37)
            .map(|i| if i % 19 < 7 { 0 } else { 255 })
            .collect();
        let image = GrayImage::new(&data, 33, 35, 37).unwrap();
        for limit in 0..1500 {
            let budget = PixelBudget::new(limit);
            let bin = Binary::<true> {
                image,
                budget: &budget,
                cache: None,
                memo: None,
                fill_cache: false,
                threshold: 127,
                inverted: false,
                local: false,
                range: [0, 255],
                radius: 24,
            };
            let mut storage = [0; 32 * FINDER_BYTES];
            let mut stats = Stats::default();
            let result = scan(bin, &mut storage, 32, 1, &mut stats, None);
            assert!(budget.used.get() <= limit);
            if result == Err(Error::PixelBudget) {
                assert_eq!(budget.used.get(), limit);
            }
        }
    }
    extern crate std;
    use super::*;

    #[test]
    fn specialization_preserves_global_pattern_results_and_partial_budgets() {
        for (width, height, stride) in [(1, 1, 1), (8, 9, 13), (97, 83, 103)] {
            let pixels: std::vec::Vec<_> = (0..stride * height)
                .map(|i| ((i * 17 + i / stride * 31) % 256) as u8)
                .collect();
            let image = GrayImage::new(&pixels, width, height, stride).unwrap();
            for inverted in [false, true] {
                for threshold in [50, 127, 200] {
                    for pitch in [0.5, 1.0, 2.0, 5.0, 10.0, 100.0] {
                        for limit in [0, 1, 2, 3, 5, 20, 48, 49, 50, 195, 196, usize::MAX] {
                            for i in 0..64 {
                                let x = i * 13 % width;
                                let y = i * 17 % height;
                                let f = Finder {
                                    p: Point {
                                        x: x as f32 + 0.125,
                                        y: y as f32 + 0.25,
                                    },
                                    pitch_x: pitch,
                                    pitch_y: pitch * 0.75,
                                    hits: 1,
                                };
                                let ab = PixelBudget::new(limit);
                                let bb = PixelBudget::new(limit);
                                let a = Binary::<true> {
                                    image,
                                    budget: &ab,
                                    cache: None,
                                    memo: None,
                                    fill_cache: false,
                                    threshold,
                                    inverted,
                                    local: false,
                                    range: [0, 255],
                                    radius: 24,
                                };
                                let b = Binary::<true> { budget: &bb, ..a };
                                assert_eq!(
                                    observation_pattern_global(a, f),
                                    observation_pattern(b, f)
                                );
                                assert_eq!(ab.used.get(), bb.used.get());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn partial_local_sum_bounds_match_full_threshold() {
        let mut state = 123u32;
        for (width, height, stride) in [(1, 1, 1), (7, 9, 11), (83, 41, 97)] {
            for spread in [5, 11, 12, 255] {
                let data: std::vec::Vec<_> = (0..height * stride)
                    .map(|_| {
                        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                        (state % (spread + 1)) as u8
                    })
                    .collect();
                let range = [*data.iter().min().unwrap(), *data.iter().max().unwrap()];
                let parent = GrayImage::new(&data, width, height, stride).unwrap();
                let image = if width > 10 {
                    parent
                        .sub_roi(Rect {
                            x: 3,
                            y: 2,
                            width: width - 6,
                            height: height - 4,
                        })
                        .unwrap()
                } else {
                    parent
                };
                for radius in [4, 5, 24, 25, 127, 128] {
                    for threshold in [0, 127, 255] {
                        for inverted in [false, true] {
                            for y in 0..image.height {
                                for x in 0..image.width {
                                    let old_budget = PixelBudget::new(usize::MAX);
                                    let new_budget = PixelBudget::new(usize::MAX);
                                    let old = Binary::<false> {
                                        image,
                                        budget: &old_budget,
                                        cache: None,
                                        memo: None,
                                        fill_cache: false,
                                        threshold,
                                        inverted,
                                        local: true,
                                        range,
                                        radius,
                                    };
                                    let new = Binary::<true> {
                                        image,
                                        budget: &new_budget,
                                        cache: None,
                                        memo: None,
                                        fill_cache: false,
                                        threshold,
                                        inverted,
                                        local: true,
                                        range,
                                        radius,
                                    };
                                    assert_eq!(old.dark(x, y), new.dark(x, y));
                                    assert!(new_budget.used.get() <= old_budget.used.get());
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut data = [200u8; 64];
        data[4 * 8 + 4] = 10;
        let image = GrayImage::new(&data, 8, 8, 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let old_budget = PixelBudget::new(usize::MAX);
                let old = Binary::<false> {
                    image,
                    budget: &old_budget,
                    cache: None,
                    memo: None,
                    fill_cache: false,
                    threshold: 105,
                    inverted: false,
                    local: true,
                    range: [*data.iter().min().unwrap(), *data.iter().max().unwrap()],
                    radius: 4,
                };
                let expected = old.dark(x, y).unwrap();
                for limit in 0..=26 {
                    let budget = PixelBudget::new(limit);
                    let memo = [const { Cell::new(0u8) }; 16];
                    let bin = Binary::<true> {
                        image,
                        budget: &budget,
                        cache: None,
                        memo: Some(&memo),
                        fill_cache: false,
                        threshold: 105,
                        inverted: false,
                        local: true,
                        range: [*data.iter().min().unwrap(), *data.iter().max().unwrap()],
                        radius: 4,
                    };
                    match bin.dark(x, y) {
                        Err(Error::PixelBudget) => {
                            assert_eq!(budget.used.get(), limit);
                            assert!(memo.iter().all(|b| b.get() == 0));
                        }
                        Ok(value) => {
                            assert_eq!(value, expected);
                            assert_eq!(bin.memo_get(x, y), Some(expected));
                        }
                        other => panic!("{other:?}"),
                    }
                    assert!(budget.used.get() <= limit);
                }
            }
        }
    }

    #[test]
    fn global_specialization_preserves_partial_budgets_and_results() {
        for (width, height, stride) in [(1, 1, 1), (8, 9, 13), (97, 83, 103)] {
            let pixels: std::vec::Vec<_> = (0..stride * height)
                .map(|i| ((i * 17 + i / stride * 31) % 256) as u8)
                .collect();
            let image = GrayImage::new(&pixels, width, height, stride).unwrap();
            for inverted in [false, true] {
                for threshold in [50, 127, 200] {
                    for expected in [0.5, 1.0, 2.0, 5.0, 10.0, 100.0] {
                        for limit in [0, 1, 2, 3, 5, 20, usize::MAX] {
                            for i in 0..64 {
                                for horizontal in [false, true] {
                                    let x = i * 13 % width;
                                    let y = i * 17 % height;
                                    let ab = PixelBudget::new(limit);
                                    let bb = PixelBudget::new(limit);
                                    let a = Binary::<true> {
                                        image,
                                        budget: &ab,
                                        cache: None,
                                        memo: None,
                                        fill_cache: false,
                                        threshold,
                                        inverted,
                                        local: false,
                                        range: [0, 255],
                                        radius: 24,
                                    };
                                    let b = Binary::<true> { budget: &bb, ..a };
                                    assert_eq!(
                                        cross_check_global(a, x, y, horizontal, expected),
                                        cross_check(b, x, y, horizontal, expected)
                                    );
                                    assert_eq!(ab.used.get(), bb.used.get());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn register_scan_matches_direct_with_stride_partial_budgets_and_helpers() {
        for (width, height, stride) in [(1, 1, 1), (8, 9, 13), (97, 83, 103)] {
            let pixels: std::vec::Vec<_> = (0..stride * height)
                .map(|i| ((i * 17 + i / stride * 31) % 256) as u8)
                .collect();
            let image = GrayImage::new(&pixels, width, height, stride).unwrap();
            for inverted in [false, true] {
                for threshold in [50, 127, 200] {
                    for step in [1, 2, 3] {
                        for limit in [
                            0,
                            1,
                            2,
                            width * height - 1,
                            width * height,
                            width * height + 1,
                            999,
                            usize::MAX,
                        ] {
                            let reference_budget = PixelBudget::new(limit);
                            let optimized_budget = PixelBudget::new(limit);
                            let reference = Binary {
                                inverted,
                                threshold,
                                ..test_binary(image, &reference_budget, false)
                            };
                            let optimized = Binary::<true> {
                                image,
                                budget: &optimized_budget,
                                cache: None,
                                memo: None,
                                fill_cache: false,
                                threshold,
                                inverted,
                                local: false,
                                range: [0, 255],
                                radius: 24,
                            };
                            let mut reference_storage = [0xaa; 32 * FINDER_BYTES + 7];
                            let mut optimized_storage = [0xaa; 32 * FINDER_BYTES + 7];
                            let mut reference_stats = Stats::default();
                            let mut optimized_stats = Stats::default();
                            let expected = scan(
                                reference,
                                &mut reference_storage,
                                32,
                                step,
                                &mut reference_stats,
                                None,
                            );
                            let actual = scan(
                                optimized,
                                &mut optimized_storage,
                                32,
                                step,
                                &mut optimized_stats,
                                None,
                            );
                            assert_eq!(
                                actual, expected,
                                "width={width}, height={height}, step={step}, limit={limit}"
                            );
                            assert_eq!(reference_budget.used.get(), optimized_budget.used.get());
                            assert_eq!(reference_stats, optimized_stats);
                            assert_eq!(reference_storage, optimized_storage);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn column_thresholds_match_direct_at_edges_odd_radii_stride_and_low_contrast() {
        let mut random = 0x839a_10efu32;
        for (width, height, stride) in [(1, 1, 1), (3, 7, 8), (83, 41, 97)] {
            for spread in [5, 11, 12, 255] {
                let pixels: std::vec::Vec<_> = (0..stride * height)
                    .map(|_| {
                        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                        (random % (spread + 1)) as u8
                    })
                    .collect();
                let parent = GrayImage::new(&pixels, width, height, stride).unwrap();
                let image = if width > 10 {
                    parent
                        .sub_roi(Rect {
                            x: 3,
                            y: 2,
                            width: width - 6,
                            height: height - 4,
                        })
                        .unwrap()
                } else {
                    parent
                };
                for radius in [4, 5, 7, 24, 25, 127, 128] {
                    for inverted in [false, true] {
                        let budget = PixelBudget::new(usize::MAX);
                        let bin = Binary::<true> {
                            image,
                            budget: &budget,
                            cache: None,
                            memo: None,
                            fill_cache: false,
                            threshold: 127,
                            inverted,
                            local: true,
                            range: [0, 255],
                            radius,
                        };
                        let mut bytes = std::vec![0xaa; image.width * 4 + 7];
                        {
                            let mut columns = ColumnSummary::new(&mut bytes[..image.width * 4]);
                            for y in 0..image.height {
                                columns.fill(bin, y).unwrap();
                                for x in 0..image.width {
                                    assert_eq!(columns.dark(bin, x, y), bin.dark(x, y),
                                        "width={width}, height={height}, radius={radius}, x={x}, y={y}");
                                }
                            }
                        }
                        assert!(bytes[image.width * 4..].iter().all(|&b| b == 0xaa));
                    }
                }
            }
        }
    }

    #[test]
    fn bulk_column_fill_matches_scalar_with_existing_budget_and_strided_roi() {
        // No padding after the final parent row; the ROI reaches that row.
        let pixels: std::vec::Vec<_> = (0..4 * 11 + 7).map(|i| (i * 37) as u8).collect();
        let parent = GrayImage::new(&pixels, 7, 5, 11).unwrap();
        let image = parent
            .sub_roi(Rect {
                x: 2,
                y: 1,
                width: 5,
                height: 4,
            })
            .unwrap();
        for radius in [4, 5, 24, 127, 128] {
            for start in [0, 17, usize::MAX - 100] {
                for remaining in [0, 1, 24, 25, 26] {
                    for y in 0..image.height {
                        let scalar_budget = PixelBudget::new(start + remaining);
                        let bulk_budget = PixelBudget::new(start + remaining);
                        scalar_budget.used.set(start);
                        bulk_budget.used.set(start);
                        let scalar = Binary {
                            radius,
                            ..test_binary(image, &scalar_budget, true)
                        };
                        let bulk = Binary::<true> {
                            image,
                            budget: &bulk_budget,
                            cache: None,
                            memo: None,
                            fill_cache: false,
                            threshold: 127,
                            inverted: false,
                            local: true,
                            range: [0, 255],
                            radius,
                        };
                        let mut expected = [0xa5; 27];
                        let mut actual = expected;
                        let reference = ColumnSummary::new(&mut expected[..20]).fill(scalar, y);
                        let result = ColumnSummary::new(&mut actual[..20]).fill(bulk, y);
                        assert_eq!(result, reference);
                        assert_eq!(actual, expected);
                        assert_eq!(bulk_budget.used.get(), scalar_budget.used.get());
                        assert_eq!(bulk_budget.used.get(), start + remaining.min(25));
                    }
                }
            }
        }
    }

    #[test]
    fn column_fills_and_center_reads_obey_partial_pixel_budgets() {
        let image = GrayImage::new(&[128; 8], 4, 2, 4).unwrap();
        for limit in 1..=21 {
            let budget = PixelBudget::new(limit);
            let bin = Binary::<true> {
                image,
                budget: &budget,
                cache: None,
                memo: None,
                fill_cache: false,
                threshold: 127,
                inverted: false,
                local: true,
                range: [0, 255],
                radius: 5,
            };
            let mut bytes = [0xaa; 16];
            let mut columns = ColumnSummary::new(&mut bytes);
            if limit < 20 {
                assert_eq!(columns.fill(bin, 0), Err(Error::PixelBudget));
            } else {
                assert_eq!(columns.fill(bin, 0), Ok(()));
                assert_eq!(
                    columns.dark(bin, 0, 0),
                    if limit == 20 {
                        Err(Error::PixelBudget)
                    } else {
                        Ok(false)
                    }
                );
            }
            assert_eq!(budget.used.get(), limit);
        }
    }

    #[test]
    fn optimized_alignment_selects_the_same_point_and_reads_less() {
        let mut random = 91234u32;
        for inverted in [false, true] {
            let mut pixels = [255; 51 * 47];
            for p in &mut pixels {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                if random % 5 == 0 {
                    *p = 0;
                }
            }
            // Two equally good patterns exercise strict score and traversal ties.
            for (cx, cy) in [(21, 23), (25, 23)] {
                for y in -2i32..=2 {
                    for x in -2i32..=2 {
                        let dark = x.abs() == 2 || y.abs() == 2 || (x == 0 && y == 0);
                        pixels[((cy + y) * 51 + cx + x) as usize] =
                            if dark ^ inverted { 0 } else { 255 };
                    }
                }
            }
            let image = GrayImage::new(&pixels, 51, 47, 51).unwrap();
            for local in [false, true] {
                for prediction in [Point { x: 23.0, y: 23.0 }, Point { x: 1.0, y: 1.0 }] {
                    let raw_budget = PixelBudget::new(usize::MAX);
                    let opt_budget = PixelBudget::new(usize::MAX);
                    let raw = Binary {
                        inverted,
                        range: [0, 255],
                        radius: 5,
                        ..test_binary(image, &raw_budget, local)
                    };
                    let opt = Binary::<true> {
                        image,
                        budget: &opt_budget,
                        cache: None,
                        memo: None,
                        fill_cache: false,
                        threshold: 127,
                        inverted,
                        local,
                        range: [0, 255],
                        radius: 5,
                    };
                    let u = Point { x: 1.0, y: 0.0 };
                    let v = Point { x: 0.0, y: 1.0 };
                    assert_eq!(
                        find_alignment(raw, prediction, u, v),
                        find_alignment(opt, prediction, u, v)
                    );
                    assert!(opt_budget.used.get() < raw_budget.used.get());
                }
            }
        }
    }

    #[test]
    fn fast_skips_dense_retry_after_low_contrast_rejection() {
        let image = GrayImage::new(&[0; 16], 4, 4, 4).unwrap();
        let config = Config::<4, 4>::default();
        let mut ds = [0; MAX_MATRIX_BYTES + 32 * FINDER_BYTES];
        let mut cs = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats::default();
        assert_eq!(
            decode_with_mode(
                image,
                &config,
                DetectionMode::Fast,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut [],
                &mut [],
                &mut stats
            ),
            Err(Error::NotFound)
        );
        assert_eq!(stats.pixel_reads, 16);
        assert_eq!(stats.passes, 0);
    }

    #[test]
    fn fast_dense_fallback_shares_budgets_and_does_not_repeat_equal_spacing() {
        let pixels = [
            0, 255, 0, 255, 255, 0, 255, 0, 0, 255, 0, 255, 255, 0, 255, 0,
        ];
        let image = GrayImage::new(&pixels, 4, 4, 4).unwrap();
        let mut ds = [0; MAX_MATRIX_BYTES + 32 * FINDER_BYTES];
        let mut cs = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats::default();
        for row_step in [1, 2, 3, 4] {
            let config = Config::<4, 4> {
                row_step,
                ..Config::default()
            };
            let result = decode_with_mode(
                image,
                &config,
                DetectionMode::Fast,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut [],
                &mut [],
                &mut stats,
            );
            assert_eq!(result, Err(Error::NotFound));
            assert_eq!(stats.passes, if row_step < 3 { 16 } else { 8 });
        }
        let config = Config::<4, 4> {
            max_pixel_reads: 129,
            ..Config::default()
        };
        assert_eq!(
            decode_with_mode(
                image,
                &config,
                DetectionMode::Fast,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut [],
                &mut [],
                &mut stats
            ),
            Err(Error::PixelBudget)
        );
        assert_eq!(stats.pixel_reads, 129);
        // Nearby global thresholds consume this small image budget before dense fallback.
        assert_eq!(stats.passes, 8);
    }

    #[test]
    fn cached_local_matches_sparse_threshold_and_reduces_scan_reads() {
        for radius in [4, 5, 24, 25, 128] {
            for (width, height, stride) in [(1, 1, 1), (8, 9, 13), (97, 83, 103)] {
                for contrast in [5, 255] {
                    let pixels: std::vec::Vec<u8> = (0..stride * height)
                        .map(|i| ((i * 17 + i / stride * 31) % (contrast + 1)) as u8)
                        .collect();
                    let parent = GrayImage::new(&pixels, width, height, stride).unwrap();
                    let image = if width > 8 {
                        parent
                            .sub_roi(Rect {
                                x: 3,
                                y: 2,
                                width: width - 6,
                                height: height - 4,
                            })
                            .unwrap()
                    } else {
                        parent
                    };
                    for inverted in [false, true] {
                        let raw_budget = PixelBudget::new(usize::MAX);
                        let cached_budget = PixelBudget::new(usize::MAX);
                        let mut bytes = std::vec![0xaa; 5 * image.width + 7];
                        {
                            let cache = RefCell::new(RowCache::new(&mut bytes));
                            let raw = Binary {
                                radius,
                                inverted,
                                ..test_binary(image, &raw_budget, true)
                            };
                            let cached = Binary {
                                budget: &cached_budget,
                                cache: Some(&cache),
                                memo: None,
                                fill_cache: true,
                                ..raw
                            };
                            for y in 0..image.height {
                                for x in 0..image.width {
                                    assert_eq!(raw.dark(x, y), cached.dark(x, y));
                                }
                            }
                            if width > 8 {
                                assert!(cached_budget.used.get() < raw_budget.used.get());
                            }
                        }
                        assert!(bytes[5 * image.width..].iter().all(|&b| b == 0xaa));
                    }
                }
            }
        }
    }
    #[test]
    fn irregular_misses_read_directly_without_replacing_scan_rows() {
        let pixels = [7; 4 * 8];
        let image = GrayImage::new(&pixels, 4, 8, 4).unwrap();
        let mut bytes = [0; 20];
        let mut cache = RowCache::new(&mut bytes);
        let budget = PixelBudget::new(21);
        for row in 0..5 {
            assert_eq!(cache.read(image, &budget, 0, row, true), Ok(7));
        }
        let tags = cache.tags;
        let order = cache.order;
        assert_eq!(cache.read(image, &budget, 0, 7, false), Ok(7));
        assert_eq!(budget.used.get(), 21);
        assert_eq!(cache.tags, tags);
        assert_eq!(cache.order, order);
        assert_eq!(cache.read(image, &budget, 3, 0, false), Ok(7));
        assert_eq!(
            cache.read(image, &budget, 0, 6, false),
            Err(Error::PixelBudget)
        );
        assert_eq!(cache.tags, tags);
        assert_eq!(cache.order, order);
    }
    #[test]
    fn incomplete_cached_fill_is_invalid_and_budget_is_exact() {
        let pixels = [0, 1, 2, 3, 4, 5, 6, 7];
        let image = GrayImage::new(&pixels, 4, 2, 4).unwrap();
        let mut bytes = [0xaa; 20];
        let mut cache = RowCache::new(&mut bytes);
        for limit in 0..4 {
            let budget = PixelBudget::new(limit);
            assert_eq!(
                cache.read(image, &budget, 0, 0, true),
                Err(Error::PixelBudget)
            );
            assert_eq!(budget.used.get(), limit);
            assert!(!cache.tags.contains(&Some(0)));
        }
        let budget = PixelBudget::new(4);
        assert_eq!(cache.read(image, &budget, 2, 0, true), Ok(2));
        assert_eq!(cache.read(image, &budget, 3, 0, true), Ok(3));
        assert_eq!(budget.used.get(), 4);
        assert_eq!(
            cache.read(image, &budget, 0, 1, true),
            Err(Error::PixelBudget)
        );
        assert!(!cache.tags.contains(&Some(1)));
    }
    #[test]
    fn rejected_geometric_sample_consumes_retry_and_preserves_decode_error() {
        let budget = PixelBudget::new(DEFAULT_MAX_PIXEL_READS);
        let image = GrayImage::new(&[255; 21 * 21], 21, 21, 21).unwrap();
        let bin = Binary::<false> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local: false,
            range: [0, 255],
            radius: 24,
        };
        let mut matrix = [0; MAX_MATRIX_BYTES];
        let mut core_scratch = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats {
            retries: 1,
            ..Stats::default()
        };
        let mut search = SearchContext {
            matrix: &mut matrix,
            core_scratch: &mut core_scratch,
            output: &mut [],
            segments: &mut [],
            stats: &mut stats,
            max_candidates: 1,
            max_retries: 2,
            last_error: Some(crate::core::Error::Version),
            strategy: SamplingStrategy::CenterThenMajority5,
            candidate_hint: None,
        };
        // The first sampled point lies outside the image.
        let outside = Homography([1.0, 0.0, -10.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert_eq!(search.attempt(bin, outside, 21), Ok(None));
        assert_eq!(search.stats.retries, 2);
        assert_eq!(search.last_error, Some(crate::core::Error::Version));
        assert_eq!(search.attempt(bin, outside, 21), Err(Error::RetryBudget));
        assert_eq!(search.stats.retries, 2);
    }
    #[test]
    fn vertical_row_summaries_match_exact_local_thresholds_and_cross_checks() {
        let mut state = 0x1293a5e7u32;
        let mut comparisons = 0;
        for (width, height, stride) in [(1, 1, 1), (9, 11, 17), (83, 131, 97)] {
            for (base, spread) in [(0, 5), (60, 11), (120, 12), (0, 255)] {
                let data: std::vec::Vec<_> = (0..height * stride)
                    .map(|_| {
                        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                        (base + state % (spread + 1)) as u8
                    })
                    .collect();
                let parent = GrayImage::new(&data, width, height, stride).unwrap();
                let image = if width > 20 {
                    parent
                        .sub_roi(Rect {
                            x: 3,
                            y: 2,
                            width: width - 6,
                            height: height - 4,
                        })
                        .unwrap()
                } else {
                    parent
                };
                for radius in [4, 5, 7, 24, 25, 127, 128] {
                    for inverted in [false, true] {
                        for threshold in [0, 127, 255] {
                            for x in [0, image.width / 2, image.width - 1] {
                                let old_budget = PixelBudget::new(usize::MAX);
                                let new_budget = PixelBudget::new(usize::MAX);
                                let old = Binary::<true> {
                                    image,
                                    budget: &old_budget,
                                    cache: None,
                                    memo: None,
                                    fill_cache: false,
                                    threshold,
                                    inverted,
                                    local: true,
                                    range: [base as u8, (base + spread) as u8],
                                    radius,
                                };
                                let new = Binary {
                                    budget: &new_budget,
                                    ..old
                                };
                                let mut cache = [0u8; 384];
                                let mut rows = AxisPacked::new(&mut cache);
                                for order in 0..3 {
                                    for i in 0..image.height {
                                        let y = match order {
                                            0 => i,
                                            1 => image.height - 1 - i,
                                            _ => (i * 37) % image.height,
                                        };
                                        assert_eq!(rows.dark(new, x, y), old.dark(x, y));
                                        comparisons += 1;
                                    }
                                }
                                assert!(new_budget.used.get() <= old_budget.used.get());
                                for y in [0, image.height / 2, image.height - 1] {
                                    let expected = cross_check_pixels(image.height, y, 3.0, |p| {
                                        old.dark(x, p)
                                    });
                                    let mut cache = [0u8; 384];
                                    assert_eq!(
                                        cross_check_vertical_packed(new, x, y, 3.0, &mut cache),
                                        expected
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        std::println!("axis threshold comparisons={comparisons}");
    }

    #[test]
    fn vertical_row_summaries_preserve_partial_fills_memo_and_budgets() {
        let data: std::vec::Vec<_> = (0..131 * 13)
            .map(|i| ((i * 37 + i / 13 * 53) % 256) as u8)
            .collect();
        let image = GrayImage::new(&data, 9, 131, 13).unwrap();
        for limit in 0..5 {
            let budget = PixelBudget::new(limit);
            let bin = Binary::<true> {
                image,
                budget: &budget,
                cache: None,
                memo: None,
                fill_cache: false,
                threshold: 127,
                inverted: false,
                local: true,
                range: [0, 255],
                radius: 5,
            };
            let mut cache = [0u8; 384];
            let mut rows = AxisPacked::new(&mut cache);
            rows.bytes[..6].copy_from_slice(&[1, 0, 123, 0, 12, 45]);
            assert!(matches!(rows.row(bin, 4, 64), Err(Error::PixelBudget)));
            assert_eq!(&rows.bytes[..6], &[1, 0, 123, 0, 12, 45]);
            assert_eq!(budget.used.get(), limit);
        }
        for y in [0, 1, 63, 64, 65, 129, 130] {
            for inverted in [false, true] {
                let expected_budget = PixelBudget::new(usize::MAX);
                let expected_bin = Binary::<true> {
                    image,
                    budget: &expected_budget,
                    cache: None,
                    memo: None,
                    fill_cache: false,
                    threshold: 127,
                    inverted,
                    local: true,
                    range: [0, 255],
                    radius: 25,
                };
                let expected = expected_bin.dark(4, y).unwrap();
                for limit in 0..=26 {
                    let budget = PixelBudget::new(limit);
                    let memo: std::vec::Vec<_> =
                        (0..(9 * 131 + 3) / 4).map(|_| Cell::new(0)).collect();
                    let bin = Binary::<true> {
                        image,
                        budget: &budget,
                        memo: Some(&memo),
                        cache: None,
                        fill_cache: false,
                        threshold: 127,
                        inverted,
                        local: true,
                        range: [0, 255],
                        radius: 25,
                    };
                    let mut cache = [0u8; 384];
                    let mut rows = AxisPacked::new(&mut cache);
                    match rows.dark(bin, 4, y) {
                        Err(Error::PixelBudget) => {
                            assert_eq!(budget.used.get(), limit);
                            assert!(memo.iter().all(|v| v.get() == 0));
                        }
                        Ok(value) => {
                            assert_eq!(value, expected);
                            let used = budget.used.get();
                            assert_eq!(rows.dark(bin, 4, y), Ok(expected));
                            assert_eq!(budget.used.get(), used);
                        }
                        other => panic!("{other:?}"),
                    }
                    assert!(budget.used.get() <= limit);
                }
            }
        }
        let last_data = std::vec![128;32768];
        let image = GrayImage::new(&last_data, 1, 32768, 1).unwrap();
        let budget = PixelBudget::new(100);
        let bin = Binary::<true> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local: true,
            range: [128, 128],
            radius: 128,
        };
        let mut cache = [0u8; 384];
        let mut rows = AxisPacked::new(&mut cache);
        assert_eq!(rows.dark(bin, 0, 32767), Ok(false));
        assert!(rows
            .bytes
            .chunks_exact(6)
            .any(|r| u16::from_le_bytes([r[0], r[1]]) == 32768));
    }
    #[test]
    fn vertical_row_summaries_scratch_boundary_and_canaries() {
        for width in [1usize, 83, 883, 884, 979] {
            let height = 83;
            let stride = width + 7;
            let data: std::vec::Vec<_> = (0..stride * height)
                .map(|i| ((i * 37 + i / stride * 53) % 256) as u8)
                .collect();
            let image = GrayImage::new(&data, width, height, stride).unwrap();
            for inverted in [false, true] {
                for radius in [5, 24, 25] {
                    let expected_budget = PixelBudget::new(usize::MAX);
                    let actual_budget = PixelBudget::new(usize::MAX);
                    let old = Binary::<true> {
                        image,
                        budget: &expected_budget,
                        cache: None,
                        memo: None,
                        fill_cache: false,
                        threshold: 127,
                        inverted,
                        local: true,
                        range: [0, 255],
                        radius,
                    };
                    let new = Binary {
                        budget: &actual_budget,
                        ..old
                    };
                    let mut old_scratch = [0xa5; MAX_MATRIX_BYTES + 7];
                    let mut new_scratch = old_scratch;
                    let mut old_storage = [0x5a; 32 * FINDER_BYTES + 7];
                    let mut new_storage = old_storage;
                    let mut old_stats = Stats::default();
                    let mut new_stats = Stats::default();
                    let a = scan(
                        old,
                        &mut old_storage,
                        32,
                        1,
                        &mut old_stats,
                        Some(&mut old_scratch[..4 * width]),
                    );
                    let b = scan(
                        new,
                        &mut new_storage,
                        32,
                        1,
                        &mut new_stats,
                        Some(&mut new_scratch[..MAX_MATRIX_BYTES]),
                    );
                    assert_eq!(a, b);
                    assert_eq!(old_stats, new_stats);
                    assert_eq!(old_storage, new_storage);
                    assert_eq!(&old_scratch[..4 * width], &new_scratch[..4 * width]);
                    assert_eq!(&new_scratch[MAX_MATRIX_BYTES..], &[0xa5; 7]);
                    if width > 883 {
                        assert_eq!(old_scratch, new_scratch);
                        assert_eq!(expected_budget.used.get(), actual_budget.used.get());
                    } else {
                        assert!(actual_budget.used.get() <= expected_budget.used.get());
                        assert!(new_scratch[4 * width + 384..].iter().all(|v| *v == 0xa5));
                    }
                }
            }
        }
    }
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn global_mask8_matches_all_thresholds_and_lanes() {
        for threshold in 0..=255u8 {
            for value in 0..=255u8 {
                for lane in 0..8 {
                    for background in [0, 127, 128, 255] {
                        let mut pixels = [background; 8];
                        pixels[lane] = value;
                        let expected = pixels
                            .iter()
                            .enumerate()
                            .fold(0u8, |bits, (i, &p)| bits | (u8::from(p <= threshold) << i));
                        assert_eq!(global_mask8(&pixels, threshold), expected);
                    }
                }
            }
        }
        let mut seed = 0x12345678u32;
        for _ in 0..50000 {
            let mut pixels = [0; 8];
            for p in &mut pixels {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *p = (seed >> 24) as u8;
            }
            let threshold = (seed >> 16) as u8;
            let expected = pixels
                .iter()
                .enumerate()
                .fold(0u8, |bits, (i, &p)| bits | (u8::from(p <= threshold) << i));
            assert_eq!(global_mask8(&pixels, threshold), expected);
        }
    }
    fn test_binary<'a>(image: GrayImage<'a>, budget: &'a PixelBudget, local: bool) -> Binary<'a> {
        Binary {
            image,
            budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local,
            range: [0, 255],
            radius: 24,
        }
    }

    #[test]
    fn local_reads_are_charged_individually_including_clamped_duplicates() {
        let image = GrayImage::new(&[0; 64], 8, 8, 8).unwrap();
        for limit in [1, 2, 24, 25, 26, 27] {
            let budget = PixelBudget::new(limit);
            let bin = test_binary(image, &budget, true);
            let expected = if limit < 26 {
                Err(Error::PixelBudget)
            } else {
                Ok(true)
            };
            assert_eq!(bin.dark(0, 0), expected);
            assert_eq!(budget.used.get(), limit.min(26));
            // A fresh decision cannot use the unused part as a complete local read.
            assert_eq!(bin.dark(0, 0), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), limit);
        }
        let budget = PixelBudget::new(1);
        assert_eq!(test_binary(image, &budget, false).dark(0, 0), Ok(true));
        assert_eq!(
            test_binary(image, &budget, false).dark(0, 0),
            Err(Error::PixelBudget)
        );
        assert_eq!(budget.used.get(), 1);
    }

    #[test]
    fn exhausted_budget_checks_before_access_and_never_overflows() {
        let image = GrayImage::new(&[255], 1, 1, 1).unwrap();
        let budget = PixelBudget::new(usize::MAX);
        budget.used.set(usize::MAX - 1);
        assert_eq!(budget.read(image, 0, 0), Ok(255));
        // Out-of-bounds coordinates would panic if the pixel were accessed.
        assert_eq!(
            budget.read(image, usize::MAX, usize::MAX),
            Err(Error::PixelBudget)
        );
        assert_eq!(budget.used.get(), usize::MAX);
    }

    #[test]
    fn all_finder_and_alignment_helpers_propagate_exhaustion() {
        let image = GrayImage::new(&[0; 64 * 64], 64, 64, 64).unwrap();
        let center = Point { x: 32.0, y: 32.0 };
        let u = Point { x: 1.0, y: 0.0 };
        let v = Point { x: 0.0, y: 1.0 };
        let f = Finder {
            p: center,
            pitch_x: 1.0,
            pitch_y: 1.0,
            hits: 1,
        };
        for local in [false, true] {
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(
                cross_check(bin, 32, 32, false, 1.0),
                Err(Error::PixelBudget)
            );
            assert_eq!(budget.used.get(), 1);
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(cross_check(bin, 32, 32, true, 1.0), Err(Error::PixelBudget));
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(finder_pattern(bin, center, u, v), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(observation_pattern(bin, f), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(find_alignment(bin, center, u, v), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
        }
    }

    #[test]
    fn sampling_exhaustion_is_not_retained_as_a_content_error() {
        let image = GrayImage::new(&[0; 21 * 21], 21, 21, 21).unwrap();
        let h = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        for local in [false, true] {
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            let mut matrix = [0; MAX_MATRIX_BYTES];
            let mut cs = [0; crate::core::SCRATCH_BYTES];
            let mut stats = Stats::default();
            let mut search = SearchContext {
                matrix: &mut matrix,
                core_scratch: &mut cs,
                output: &mut [],
                segments: &mut [],
                stats: &mut stats,
                max_candidates: 1,
                max_retries: 2,
                last_error: Some(crate::core::Error::Version),
                strategy: SamplingStrategy::Center,
                candidate_hint: None,
            };
            assert_eq!(search.attempt(bin, h, 21), Err(Error::PixelBudget));
            assert_eq!(search.last_error, Some(crate::core::Error::Version));
            assert_eq!(search.stats.retries, 1);
            assert_eq!(budget.used.get(), 1);
        }
    }
    #[test]
    fn candidate_and_version_stop_during_alignment_before_sampling() {
        let bits = include_bytes!("../../../fixtures/v2.bin");
        let mut pixels = [255; 33 * 33];
        for y in 0..25 {
            for x in 0..25 {
                if bits[(y * 25 + x) / 8] & (128 >> ((y * 25 + x) % 8)) != 0 {
                    pixels[(y + 4) * 33 + x + 4] = 0;
                }
            }
        }
        let image = GrayImage::new(&pixels, 33, 33, 33).unwrap();
        let make = |x, y| Finder {
            p: Point { x, y },
            pitch_x: 1.0,
            pitch_y: 1.0,
            hits: 1,
        };
        let finders = [make(7.5, 7.5), make(25.5, 7.5), make(7.5, 25.5)];
        // Three 7x7 checks complete; one alignment read completes before exhaustion.
        let budget = PixelBudget::new(3 * 49 + 1);
        let bin = test_binary(image, &budget, false);
        let mut matrix = [0; MAX_MATRIX_BYTES];
        let mut cs = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats::default();
        let mut search = SearchContext {
            matrix: &mut matrix,
            core_scratch: &mut cs,
            output: &mut [],
            segments: &mut [],
            stats: &mut stats,
            max_candidates: 1,
            max_retries: 2,
            last_error: Some(crate::core::Error::Version),
            strategy: SamplingStrategy::Center,
            candidate_hint: None,
        };
        assert_eq!(search.candidate(bin, finders), Err(Error::PixelBudget));
        assert_eq!(budget.used.get(), 148);
        assert_eq!(search.stats.candidates, 1);
        assert_eq!(search.stats.retries, 0);
        assert_eq!(search.last_error, Some(crate::core::Error::Version));
    }
    #[test]
    fn five_votes_charge_duplicates_all_reads_and_partial_local_decisions() {
        let image = GrayImage::new(&[0], 1, 1, 1).unwrap();
        // Every projection maps to the same pixel; duplicates must still be read.
        let h = Homography([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        for local in [false, true] {
            let total = 21 * 21 * 5 * if local { 26 } else { 1 };
            for limit in [1, 25, 26, 129, 130, total - 1, total] {
                let budget = PixelBudget::new(limit);
                let bin = test_binary(image, &budget, local);
                let mut matrix = [0; MAX_MATRIX_BYTES];
                let mut cs = [0; crate::core::SCRATCH_BYTES];
                let mut stats = Stats::default();
                let mut search = SearchContext {
                    matrix: &mut matrix,
                    core_scratch: &mut cs,
                    output: &mut [],
                    segments: &mut [],
                    stats: &mut stats,
                    max_candidates: 1,
                    max_retries: 1,
                    last_error: None,
                    strategy: SamplingStrategy::CenterThenMajority5,
                    candidate_hint: None,
                };
                assert_eq!(
                    search.sample::<true>(bin, h, 21),
                    if limit < total {
                        Err(Error::PixelBudget)
                    } else {
                        Ok(SampleOutcome::Complete)
                    }
                );
                assert_eq!(budget.used.get(), limit.min(total));
                assert_eq!(search.stats.retries, 1);
                assert_eq!(search.last_error, None);
            }
        }
    }

    #[test]
    fn invalid_projection_rejects_majority_without_clamping() {
        let image = GrayImage::new(&[0; 21 * 21], 21, 21, 21).unwrap();
        for h in [
            Homography([0.6, 0.0, -0.2, 0.0, 0.6, 0.0, 0.0, 0.0]),
            Homography([f32::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
        ] {
            let budget = PixelBudget::new(10000);
            let mut matrix = [0; MAX_MATRIX_BYTES];
            let mut cs = [0; crate::core::SCRATCH_BYTES];
            let mut stats = Stats::default();
            let mut search = SearchContext {
                matrix: &mut matrix,
                core_scratch: &mut cs,
                output: &mut [],
                segments: &mut [],
                stats: &mut stats,
                max_candidates: 1,
                max_retries: 2,
                last_error: None,
                strategy: SamplingStrategy::CenterThenMajority5,
                candidate_hint: None,
            };
            assert_eq!(
                search.sample::<true>(test_binary(image, &budget, false), h, 21),
                Ok(SampleOutcome::InvalidGeometry)
            );
            assert_eq!(budget.used.get(), if h.0[0].is_nan() { 0 } else { 1 });
            assert_eq!(search.stats.retries, 1);
        }
        assert!(track_hint([Point { x: 1.0, y: 1.0 }; 4], 21, image).is_none());
        assert!(track_hint(
            [Point {
                x: f32::INFINITY,
                y: 1.0
            }; 4],
            21,
            image
        )
        .is_none());
        for error in [
            crate::core::Error::Format,
            crate::core::Error::Version,
            crate::core::Error::InvalidMatrix,
        ] {
            assert!(!candidate_content_error(error));
        }
    }
    #[test]
    fn every_tracker_state_preserves_itself_on_terminal_errors() {
        let file = include_bytes!("../../../fixtures/v1.pgm");
        let offset = file
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(2)
            .unwrap()
            .0
            + 1;
        let image = GrayImage::new(&file[offset..], 145, 145, 145).unwrap();
        let hint = TrackHint {
            roi: Rect {
                x: 0,
                y: 0,
                width: 145,
                height: 145,
            },
            image_width: 145,
            image_height: 145,
        };
        let config = DefaultConfig::default();
        let mut ds = [0; MAX_MATRIX_BYTES + 32 * FINDER_BYTES];
        let mut cs = [0; crate::core::SCRATCH_BYTES];
        let mut output = [0; 16];
        let mut segments = [Segment::EMPTY; 4];
        for state in [
            TrackerState::Searching,
            TrackerState::Candidate { hint, misses: 1 },
            TrackerState::Tracking { hint },
        ] {
            let mut tracker = Tracker { state };
            let mut stats = Stats::default();
            let small = DefaultConfig {
                max_pixel_reads: 1,
                ..config
            };
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &small,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats
                ),
                Err(Error::PixelBudget)
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.pixel_reads, 1);
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut [],
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats
                ),
                Err(Error::ScratchTooSmall)
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats, Stats::default());
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut [],
                    &mut segments,
                    &mut stats
                ),
                Err(Error::Decode(crate::core::Error::OutputTooSmall))
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.retries, 1);
            assert_eq!(
                tracker.decode_with_stats(
                    image,
                    &config,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut [],
                    &mut stats
                ),
                Err(Error::Decode(crate::core::Error::SegmentsTooSmall))
            );
            assert_eq!(tracker.state(), state);
            assert_eq!(stats.retries, 1);
        }
    }
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod memo_tests {
    extern crate std;
    use super::*;

    #[test]
    fn memo_exact_local_bits_columns_roi_stride_inversion_odd_radius_and_clamping() {
        for (width, height, stride) in [(1, 1, 1), (8, 9, 13), (97, 83, 103)] {
            for contrast in [5usize, 255] {
                let pixels: std::vec::Vec<u8> = (0..stride * height)
                    .map(|i| ((i * 17 + i / stride * 31) % (contrast + 1)) as u8)
                    .collect();
                let parent = GrayImage::new(&pixels, width, height, stride).unwrap();
                let image = if width > 8 {
                    parent
                        .sub_roi(Rect {
                            x: 3,
                            y: 2,
                            width: width - 6,
                            height: height - 4,
                        })
                        .unwrap()
                } else {
                    parent
                };
                for radius in [4, 5, 24, 25, 128] {
                    let budget = PixelBudget::new(usize::MAX);
                    let reference_budget = PixelBudget::new(usize::MAX);
                    let mut bytes = std::vec![0u8; (image.width * image.height).div_ceil(4)];
                    let memo = Cell::from_mut(bytes.as_mut_slice()).as_slice_of_cells();
                    let reference = Binary::<true> {
                        image,
                        budget: &reference_budget,
                        cache: None,
                        memo: None,
                        fill_cache: false,
                        threshold: 127,
                        inverted: false,
                        local: true,
                        range: [0, 255],
                        radius,
                    };
                    let bin = Binary {
                        budget: &budget,
                        memo: Some(memo),
                        ..reference
                    };
                    let mut columns_bytes = std::vec![0u8; image.width * 4];
                    let mut columns = ColumnSummary::new(&mut columns_bytes);
                    for y in 0..image.height {
                        columns.fill(bin, y).unwrap();
                        for x in 0..image.width {
                            let expected = reference.dark(x, y).unwrap();
                            let actual = if (x + y) % 2 == 0 {
                                bin.dark(x, y)
                            } else {
                                columns.dark(bin, x, y)
                            }
                            .unwrap();
                            assert_eq!(actual, expected);
                            let used = budget.used.get();
                            assert_eq!(bin.dark(x, y), Ok(expected));
                            assert_eq!(columns.dark(bin, x, y), Ok(expected));
                            assert_eq!(
                                Binary {
                                    inverted: true,
                                    ..bin
                                }
                                .dark(x, y),
                                Ok(!expected)
                            );
                            assert_eq!(budget.used.get(), used);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn exhausted_partial_local_decision_never_publishes_cache_entry() {
        let image = GrayImage::new(&[0, 255, 255, 0], 2, 2, 2).unwrap();
        for limit in 0..=26 {
            let budget = PixelBudget::new(limit);
            let mut byte = [0u8];
            let memo = Cell::from_mut(byte.as_mut_slice()).as_slice_of_cells();
            let bin = Binary::<true> {
                image,
                budget: &budget,
                cache: None,
                memo: Some(memo),
                fill_cache: false,
                threshold: 127,
                inverted: false,
                local: true,
                range: [0, 255],
                radius: 5,
            };
            // The first five clamped samples already establish contrast and a
            // positive sum for the zero-valued center: six input reads suffice.
            if limit < 6 {
                assert_eq!(bin.dark(0, 0), Err(Error::PixelBudget));
                assert_eq!(bin.memo_get(0, 0), None);
                assert_eq!(memo[0].get(), 0);
            } else {
                let result = bin.dark(0, 0).unwrap();
                assert_eq!(bin.memo_get(0, 0), Some(result));
                assert_eq!(bin.dark(0, 0), Ok(result));
            }
            assert_eq!(budget.used.get(), limit.min(6));
        }
    }

    #[test]
    fn memo_sizing_validates_dimensions_and_retains_minimal_modes() {
        let config = DefaultConfig::default();
        let base = config.scratch_bytes().unwrap();
        for (w, h) in [(1, 1), (3, 7), (640, 640)] {
            assert_eq!(
                config.scratch_bytes_for_mode(DetectionMode::Fast, w, h),
                Ok(base + (w * h).div_ceil(4))
            );
            assert_eq!(
                config.scratch_bytes_for_mode(DetectionMode::Direct, w, h),
                Ok(base)
            );
        }
        for (w, h) in [(0, 1), (1, 0), (641, 1), (1, 641), (usize::MAX, usize::MAX)] {
            assert_eq!(
                config.scratch_bytes_for_mode(DetectionMode::Fast, w, h),
                Err(Error::InvalidConfig)
            );
        }
    }
}

#[cfg(test)]
mod region_reuse_tests {
    extern crate std;
    use super::*;
    #[allow(clippy::too_many_arguments)]
    fn reference_region<const W: usize, const H: usize>(
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut Stats,
        budget: &PixelBudget,
        mut candidate_hint: Option<&mut Option<([Point; 4], usize)>>,
        cache: Option<&mut [u8]>,
        mode: DetectionMode,
    ) -> Result<Detection, Error> {
        let mut state = RegionState::default();
        // Dedicated row-cache APIs preserve their input-read order and cache-fill
        // semantics; public execution modes use the optimized uncached path below.
        if cache.is_some() {
            return decode_inner::<W, H, false>(
                image,
                config,
                strategy,
                scratch,
                core_scratch,
                output,
                segments,
                stats,
                budget,
                candidate_hint,
                cache,
                false,
                &mut state,
            );
        }
        let coarse = Config {
            row_step: if mode == DetectionMode::Fast {
                config.row_step.max(3)
            } else {
                config.row_step
            },
            ..*config
        };
        let passes_before = stats.passes;
        let result = decode_inner::<W, H, true>(
            image,
            &coarse,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            budget,
            candidate_hint.as_deref_mut(),
            None,
            mode == DetectionMode::Fast,
            &mut state,
        );
        match result {
            Err(error)
                if coarse.row_step != config.row_step
                    && stats.passes > passes_before
                    && recognition_failure(error) =>
            {
                state = RegionState::default();
                decode_inner::<W, H, true>(
                    image,
                    config,
                    strategy,
                    scratch,
                    core_scratch,
                    output,
                    segments,
                    stats,
                    budget,
                    candidate_hint,
                    None,
                    mode == DetectionMode::Fast,
                    &mut state,
                )
            }
            result => result,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        image: GrayImage<'_>,
        config: &DefaultConfig,
        scratch: &mut [u8],
        reference: bool,
    ) -> (Result<Detection, Error>, Stats, std::vec::Vec<u8>) {
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let mut output = [0; 7089];
        let mut segments = [Segment::EMPTY; 64];
        let mut stats = Stats::default();
        let budget = PixelBudget::new(config.max_pixel_reads);
        let result = if reference {
            reference_region(
                image,
                config,
                SamplingStrategy::Center,
                scratch,
                &mut core,
                &mut output,
                &mut segments,
                &mut stats,
                &budget,
                None,
                None,
                DetectionMode::Fast,
            )
        } else {
            decode_region(
                image,
                config,
                SamplingStrategy::Center,
                scratch,
                &mut core,
                &mut output,
                &mut segments,
                &mut stats,
                &budget,
                None,
                None,
                DetectionMode::Fast,
            )
        };
        stats.pixel_reads = budget.used.get();
        let payload = result
            .as_ref()
            .ok()
            .map(|f| output[..f.decoded.payload_len].to_vec())
            .unwrap_or_default();
        (result, stats, payload)
    }
    fn normalized(result: Result<Detection, Error>) -> Result<Detection, Error> {
        result.map(|mut d| {
            d.stats.pixel_reads = 0;
            d
        })
    }
    #[test]
    fn reuse_matches_fresh_passes_and_reduces_reads_after_full_search() {
        let config = DefaultConfig {
            max_pixel_reads: usize::MAX,
            ..DefaultConfig::default()
        };
        let mut fewer = false;
        for (width, height, stride) in [(4, 4, 7), (32, 29, 41), (64, 48, 71)] {
            for inverted in [false, true] {
                let pixels: std::vec::Vec<_> = (0..stride * height)
                    .map(|i| ((32 + (i % stride) * 3) % 256) as u8 ^ if inverted { 255 } else { 0 })
                    .collect();
                let image = GrayImage::new(&pixels, width, height, stride).unwrap();
                let size = config
                    .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
                    .unwrap();
                let mut ds = std::vec![0xff;size];
                let mut base = std::vec![0xff;size];
                let (actual, mut stats, payload) = run(image, &config, &mut ds, false);
                let (expected, mut old, old_payload) = run(image, &config, &mut base, true);
                assert_eq!(normalized(actual), normalized(expected));
                assert_eq!(payload, old_payload);
                let min = (0..height)
                    .flat_map(|y| (0..width).map(move |x| image.at(x, y)))
                    .min()
                    .unwrap();
                let max = (0..height)
                    .flat_map(|y| (0..width).map(move |x| image.at(x, y)))
                    .max()
                    .unwrap();
                assert_eq!(stats.passes, if max - min >= 16 { 16 } else { 8 });
                assert!(stats.pixel_reads <= old.pixel_reads);
                fewer |= stats.pixel_reads < old.pixel_reads;
                stats.pixel_reads = 0;
                old.pixel_reads = 0;
                assert_eq!(stats, old);
            }
        }
        assert!(fewer);
    }
    #[test]
    fn reuse_is_scoped_to_roi_and_call_with_poisoned_scratch_and_finite_budgets() {
        let mut reused = std::vec![0x55;100000];
        for (width, height, stride) in [(4, 4, 7), (64, 48, 71), (32, 29, 41), (4, 4, 4)] {
            for radius in [4, 25, 128] {
                for inverted in [false, true] {
                    let pixels: std::vec::Vec<_> = (0..stride * height)
                        .map(|i| {
                            ((32 + (i % stride) * 3 + i / stride) % 256) as u8
                                ^ if inverted { 255 } else { 0 }
                        })
                        .collect();
                    let image = GrayImage::new(&pixels, width, height, stride).unwrap();
                    let config = DefaultConfig {
                        local_radius: radius,
                        ..DefaultConfig::default()
                    };
                    for limit in [1, width * height, 2000, 10000, usize::MAX] {
                        let cfg = DefaultConfig {
                            max_pixel_reads: limit,
                            ..config
                        };
                        let size = cfg
                            .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
                            .unwrap();
                        reused[size..size + 7].fill(0xa5);
                        let mut fresh = std::vec![0xff;size];
                        let a = run(image, &cfg, &mut reused[..size], false);
                        let b = run(image, &cfg, &mut fresh, false);
                        assert_eq!(a, b);
                        assert!(a.1.pixel_reads <= limit);
                        if a.0 == Err(Error::PixelBudget) {
                            assert_eq!(a.1.pixel_reads, limit);
                        }
                        assert!(reused[size..size + 7].iter().all(|&v| v == 0xa5));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod sampling_specialization_tests {
    extern crate std;
    use super::*;
    fn sample<const OPT: bool, const MAJORITY: bool>(
        image: GrayImage<'_>,
        h: Homography,
        side: usize,
        threshold: u8,
        inverted: bool,
        limit: usize,
    ) -> (Result<bool, Error>, Stats, std::vec::Vec<u8>) {
        let budget = PixelBudget::new(limit);
        let bin = Binary::<OPT> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold,
            inverted,
            local: false,
            range: [0, 255],
            radius: 24,
        };
        let mut matrix = std::vec![0xa5; MAX_MATRIX_BYTES];
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats::default();
        let result = SearchContext::<OPT> {
            matrix: &mut matrix,
            core_scratch: &mut core,
            output: &mut [],
            segments: &mut [],
            stats: &mut stats,
            max_candidates: 1,
            max_retries: 1,
            last_error: None,
            strategy: SamplingStrategy::Center,
            candidate_hint: None,
        }
        .sample::<MAJORITY>(bin, h, side)
        .map(|v| v == SampleOutcome::Complete);
        stats.pixel_reads = budget.used.get();
        (result, stats, matrix)
    }
    fn sample_reference<const MAJORITY: bool>(
        image: GrayImage<'_>,
        h: Homography,
        side: usize,
        threshold: u8,
        inverted: bool,
        limit: usize,
    ) -> (Result<bool, Error>, Stats, std::vec::Vec<u8>) {
        let budget = PixelBudget::new(limit);
        let bin = Binary::<false> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold,
            inverted,
            local: false,
            range: [0, 255],
            radius: 24,
        };
        let mut matrix = std::vec![0xa5; MAX_MATRIX_BYTES];
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let mut stats = Stats {
            retries: 1,
            ..Stats::default()
        };
        let result = SearchContext::<false> {
            matrix: &mut matrix,
            core_scratch: &mut core,
            output: &mut [],
            segments: &mut [],
            stats: &mut stats,
            max_candidates: 1,
            max_retries: 1,
            last_error: None,
            strategy: SamplingStrategy::Center,
            candidate_hint: None,
        }
        .sample_pixels_reference::<MAJORITY>(h, side, |p| bin.point(p));
        stats.pixel_reads = budget.used.get();
        (result, stats, matrix)
    }
    #[test]
    fn packed_sampling_preserves_scalar_prefix_on_every_partial_byte() {
        let pixels: std::vec::Vec<_> = (0..61 * 48)
            .map(|i| ((i * 37 + i / 61 * 13) % 256) as u8)
            .collect();
        let image = GrayImage::new(&pixels[..47 * 61 + 53], 53, 48, 61).unwrap();
        for h in [
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            Homography([0.8, 0.1, 2.0, -0.1, 0.9, 3.0, 0.001, -0.002]),
            Homography([1.0, 0.0, -0.5, 0.0, 1.0, -0.5, 0.0, 0.0]),
            Homography([1.0, 0.0, 52.5, 0.0, 1.0, 47.5, 0.0, 0.0]),
            Homography([f32::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            Homography([f32::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
        ] {
            for side in [1, 2, 3, 7, 8, 9, 21, 41] {
                for threshold in [0, 127, 255] {
                    for inverted in [false, true] {
                        for limit in (0..=85).chain([
                            side * side - 1,
                            side * side,
                            side * side * 5 - 1,
                            usize::MAX,
                        ]) {
                            assert_eq!(
                                sample::<true, false>(image, h, side, threshold, inverted, limit),
                                sample_reference::<false>(
                                    image, h, side, threshold, inverted, limit
                                )
                            );
                            assert_eq!(
                                sample::<true, true>(image, h, side, threshold, inverted, limit),
                                sample_reference::<true>(
                                    image, h, side, threshold, inverted, limit
                                )
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn global_sampling_matches_generic_at_boundaries_and_partial_budgets() {
        let pixels: std::vec::Vec<_> = (0..61 * 48)
            .map(|i| ((i * 37 + i / 61 * 13) % 256) as u8)
            .collect();
        // No padding is required after the final ROI row.
        let image = GrayImage::new(&pixels[..47 * 61 + 53], 53, 48, 61).unwrap();
        for h in [
            Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            Homography([0.8, 0.1, 2.0, -0.1, 0.9, 3.0, 0.001, -0.002]),
            Homography([1.0, 0.0, -0.5, 0.0, 1.0, -0.5, 0.0, 0.0]),
            Homography([1.0, 0.0, 52.5, 0.0, 1.0, 47.5, 0.0, 0.0]),
            Homography([f32::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            Homography([f32::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
        ] {
            for side in [21, 41] {
                for threshold in [0, 127, 255] {
                    for inverted in [false, true] {
                        for limit in [
                            0,
                            1,
                            4,
                            5,
                            side * side - 1,
                            side * side,
                            5 * side * side - 1,
                            usize::MAX,
                        ] {
                            assert_eq!(
                                sample::<true, false>(image, h, side, threshold, inverted, limit),
                                sample::<false, false>(image, h, side, threshold, inverted, limit)
                            );
                            assert_eq!(
                                sample::<true, true>(image, h, side, threshold, inverted, limit),
                                sample::<false, true>(image, h, side, threshold, inverted, limit)
                            );
                        }
                    }
                }
            }
        }
    }
}

// Append this module to detect.rs. It uses no repository fixture files.
#[cfg(test)]
mod fast_refinement_policy_tests {
    use super::*;

    #[test]
    fn nearby_global_passes_gate_on_range_and_preserve_terminal_pixel_budgets() {
        const BASE: usize = MAX_MATRIX_BYTES + 32 * FINDER_BYTES;
        let mut scratch = [0xff; BASE + 4];
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let config = Config::<4, 4> {
            row_step: 3,
            ..Config::default()
        };

        // Four columns cannot contain the five positive finder-pattern runs.
        // Equal configured/coarse spacing also rules out a dense fallback.
        for span in [15u8, 16] {
            let pixels = [
                0, span, 0, span, span, 0, span, 0, 0, span, 0, span, span, 0, span, 0,
            ];
            let image = GrayImage::new(&pixels, 4, 4, 4).unwrap();
            for mode in [DetectionMode::Direct, DetectionMode::Fast] {
                for length in [BASE, BASE + 4] {
                    scratch.fill(0xff);
                    let mut stats = Stats::default();
                    assert_eq!(
                        decode_with_mode(
                            image,
                            &config,
                            mode,
                            SamplingStrategy::Center,
                            &mut scratch[..length],
                            &mut core,
                            &mut [],
                            &mut [],
                            &mut stats,
                        ),
                        Err(Error::NotFound)
                    );
                    assert_eq!(
                        stats.passes,
                        if mode == DetectionMode::Fast && span == 16 {
                            8
                        } else {
                            4
                        },
                        "mode={mode:?} span={span} scratch={length}"
                    );
                    assert_eq!(stats.candidates, 0);
                    assert_eq!(stats.retries, 0);
                }
            }
        }

        let pixels = [
            0, 255, 0, 255, 255, 0, 255, 0, 0, 255, 0, 255, 255, 0, 255, 0,
        ];
        let image = GrayImage::new(&pixels, 4, 4, 4).unwrap();
        // Min/max completes after the first 4-read row; each global pass scans rows 0 and 3
        // for eight reads. Stop one read into each of the four new passes.
        for (limit, passes) in [(21, 3), (29, 4), (37, 5), (45, 6)] {
            for length in [BASE, BASE + 4] {
                let limited = Config {
                    max_pixel_reads: limit,
                    ..config
                };
                scratch.fill(0xff);
                let mut stats = Stats::default();
                assert_eq!(
                    decode_with_mode(
                        image,
                        &limited,
                        DetectionMode::Fast,
                        SamplingStrategy::Center,
                        &mut scratch[..length],
                        &mut core,
                        &mut [],
                        &mut [],
                        &mut stats,
                    ),
                    Err(Error::PixelBudget)
                );
                assert_eq!(stats.pixel_reads, limit);
                assert_eq!(stats.passes, passes);
                assert_eq!(stats.candidates, 0);
                assert_eq!(stats.retries, 0);
                // Global decisions must neither initialize nor populate memo.
                assert!(scratch[BASE..].iter().all(|&byte| byte == 0xff));
            }
        }
    }

    #[test]
    fn nearby_global_thresholds_restore_midrange_before_local_memo_population() {
        const BASE: usize = MAX_MATRIX_BYTES + 32 * FINDER_BYTES;
        let mut pixels = [130u8; 32 * 32];
        pixels[0] = 0;
        pixels[32 * 32 - 1] = 255;
        let image = GrayImage::new(&pixels, 32, 32, 32).unwrap();
        let config = Config::<32, 32> {
            row_step: 3,
            local_radius: 4,
            ..Config::default()
        };
        let mut scratch = [0xff; BASE + 32 * 32 / 4];
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let budget = PixelBudget::new(config.max_pixel_reads);
        let mut stats = Stats::default();
        let mut state = RegionState::default();
        assert_eq!(
            decode_inner::<32, 32, true>(
                image,
                &config,
                SamplingStrategy::Center,
                &mut scratch,
                &mut core,
                &mut [],
                &mut [],
                &mut stats,
                &budget,
                None,
                None,
                true,
                &mut state,
            ),
            Err(Error::NotFound)
        );
        assert_eq!(stats.passes, 8);
        assert_eq!(state.range, Some([0, 255]));
        assert!(state.memo_ready);

        // Row 15 is scanned at step 3. Every radius-4 sample near (16, 15)
        // is 130, so contrast is zero and the original midrange 127 decides.
        // Accidentally retaining the preceding high threshold 142 would
        // classify this point as dark instead of light.
        let (x, y) = (16, 15);
        let reference_budget = PixelBudget::new(usize::MAX);
        let reference = Binary::<false> {
            image,
            budget: &reference_budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local: true,
            range: [0, 255],
            radius: 4,
        };
        let expected = reference.dark(x, y).unwrap();
        assert!(!expected);
        let index = y * image.width + x;
        let stored = (scratch[BASE + index / 4] >> ((index % 4) * 2)) & 3;
        assert_eq!(stored, if expected { 1 } else { 2 });
    }
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod header_prefilter_tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn run(
        reference: bool,
        image: GrayImage<'_>,
        h: Homography,
        side: usize,
        strategy: SamplingStrategy,
        prior: Option<crate::core::Error>,
        limit: usize,
        retry_limit: usize,
    ) -> (
        Result<Option<Detection>, Error>,
        Stats,
        Option<crate::core::Error>,
        Option<([Point; 4], usize)>,
        Vec<u8>,
        Vec<Segment>,
    ) {
        let budget = PixelBudget::new(limit);
        let bin = Binary::<true> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local: false,
            range: [0, 255],
            radius: 24,
        };
        let mut matrix = [0xa5; MAX_MATRIX_BYTES];
        let mut core = [0; crate::core::SCRATCH_BYTES];
        let mut output = [0; 7089];
        let mut segments = [Segment::EMPTY; 64];
        let mut stats = Stats::default();
        let mut hint = None;
        let mut search = SearchContext::<true> {
            matrix: &mut matrix,
            core_scratch: &mut core,
            output: &mut output,
            segments: &mut segments,
            stats: &mut stats,
            max_candidates: 4096,
            max_retries: retry_limit,
            last_error: prior,
            strategy,
            candidate_hint: Some(&mut hint),
        };
        let result = if reference {
            search.attempt_reference(bin, h, side)
        } else {
            search.attempt(bin, h, side)
        };
        let error = search.last_error;
        let (payload_len, segment_len) = result
            .as_ref()
            .ok()
            .and_then(|r| r.as_ref())
            .map_or((0, 0), |d| (d.decoded.payload_len, d.decoded.segment_count));
        // The input-read count may change; every other result/counter must match.
        assert!(budget.used.get() <= limit);
        (
            result,
            stats,
            error,
            hint,
            output[..payload_len].to_vec(),
            segments[..segment_len].to_vec(),
        )
    }
    #[test]
    fn prefilter_preserves_results_errors_hints_retries_and_terminal_limits() {
        let stride = 61;
        let size = 53;
        let mut images = Vec::new();
        let random: Vec<_> = (0..stride * size)
            .map(|i| ((i * 37 + i / stride * 13) % 256) as u8)
            .collect();
        images.push(random);
        for (side, bits) in [
            (21, include_bytes!("../../../fixtures/v1.bin").as_slice()),
            (45, include_bytes!("../../../fixtures/v7.bin").as_slice()),
        ] {
            let mut pixels = std::vec![255;stride*size];
            for y in 0..side {
                for x in 0..side {
                    let i = y * side + x;
                    pixels[y * stride + x] = if bits[i / 8] & (128 >> (i % 8)) != 0 {
                        0
                    } else {
                        255
                    };
                }
            }
            images.push(pixels);
        }
        for pixels in images {
            let image =
                GrayImage::new(&pixels[..(size - 1) * stride + size], size, size, stride).unwrap();
            for h in [
                Homography([1., 0., 0., 0., 1., 0., 0., 0.]),
                Homography([0.8, 0.1, 2., -0.1, 0.9, 3., 0.001, -0.002]),
                Homography([1., 0., -0.5, 0., 1., -0.5, 0., 0.]),
                Homography([1., 0., 52.5, 0., 1., 47.5, 0., 0.]),
                Homography([1., 0., 0., 0., 1., 0., -0.04, 0.]),
                Homography([f32::NAN, 0., 0., 0., 1., 0., 0., 0.]),
            ] {
                for side in [21, 25, 41, 45, 49] {
                    for strategy in [
                        SamplingStrategy::Center,
                        SamplingStrategy::CenterThenMajority5,
                    ] {
                        for prior in [
                            None,
                            Some(crate::core::Error::Format),
                            Some(crate::core::Error::Version),
                            Some(crate::core::Error::ReedSolomon),
                        ] {
                            for limit in [
                                0,
                                1,
                                30,
                                31,
                                32,
                                side * side - 1,
                                side * side,
                                side * side + 31 - 1,
                                side * side + 31,
                                side * side + 67,
                                5 * (side * side + 67) - 1,
                                5 * (side * side + 67),
                                usize::MAX,
                            ] {
                                for retry_limit in [1, 2, 8] {
                                    assert_eq!(run(true,image,h,side,strategy,prior,limit,retry_limit),run(false,image,h,side,strategy,prior,limit,retry_limit),"side={side} strategy={strategy:?} prior={prior:?} limit={limit} retry={retry_limit}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod header_reserve_review_tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn set(bits: &mut [u8], side: usize, x: usize, y: usize, dark: bool) {
        let index = y * side + x;
        let mask = 128 >> (index % 8);
        if dark {
            bits[index / 8] |= mask;
        } else {
            bits[index / 8] &= !mask;
        }
    }

    // Independent polynomial division, rather than calling the production BCH
    // generator, provides a valid version-8 word for a version-7 matrix.
    fn version_word(version: u32) -> u32 {
        let mut remainder = version << 12;
        for bit in (12..18).rev() {
            if remainder & (1 << bit) != 0 {
                remainder ^= 0x1f25 << (bit - 12);
            }
        }
        (version << 12) | remainder
    }

    fn wrong_version_bits() -> Vec<u8> {
        let mut bits = include_bytes!("../../../fixtures/v7.bin").to_vec();
        let word = version_word(8);
        for i in 0..18 {
            let x = 45 - 11 + i % 3;
            let y = i / 3;
            let dark = word & (1 << i) != 0;
            set(&mut bits, 45, x, y, dark);
            set(&mut bits, 45, y, x, dark);
        }
        bits
    }

    fn transpose(bits: &[u8], side: usize) -> Vec<u8> {
        let mut output = std::vec![0; (side * side).div_ceil(8)];
        for y in 0..side {
            for x in 0..side {
                let index = y * side + x;
                set(
                    &mut output,
                    side,
                    y,
                    x,
                    bits[index / 8] & (128 >> (index % 8)) != 0,
                );
            }
        }
        output
    }

    fn core_error(bits: &[u8], side: usize) -> crate::core::Error {
        crate::core::decode(
            Matrix::new(bits, side).unwrap(),
            &mut [0xa5; crate::core::SCRATCH_BYTES],
            &mut [0; 7089],
            &mut [Segment::EMPTY; 64],
        )
        .unwrap_err()
    }

    #[test]
    fn valid_format_wrong_version_preserves_original_orientation_error() {
        let bits = wrong_version_bits();
        let matrix = Matrix::new(&bits, 45).unwrap();
        // Format modules are unchanged from the valid fixture, while both
        // version copies contain a valid but incompatible version-8 codeword.
        assert_eq!(core_error(&bits, 45), crate::core::Error::Version);
        assert_eq!(
            crate::core::header_error(matrix),
            Some(crate::core::Error::Version)
        );
        let mirrored = transpose(&bits, 45);
        assert_eq!(
            crate::core::header_error(Matrix::new(&mirrored, 45).unwrap()),
            Some(core_error(&mirrored, 45))
        );
        // A valid mirrored header must remain eligible for the full decoder.
        let valid = include_bytes!("../../../fixtures/v7.bin");
        let valid_mirrored = transpose(valid, 45);
        assert_eq!(
            crate::core::header_error(Matrix::new(valid, 45).unwrap()),
            None
        );
        assert_eq!(
            crate::core::header_error(Matrix::new(&valid_mirrored, 45).unwrap()),
            None
        );
    }

    fn render(
        bits: &[u8],
        side: usize,
        scale: usize,
        whiten_centers: bool,
    ) -> (Vec<u8>, usize, usize) {
        let width = side * scale;
        let stride = width + 7;
        let mut pixels = std::vec![0xa5; (width - 1) * stride + width];
        for y in 0..width {
            for x in 0..width {
                let index = (y / scale) * side + x / scale;
                let dark = bits[index / 8] & (128 >> (index % 8)) != 0;
                pixels[y * stride + x] = if whiten_centers && x % scale == 1 && y % scale == 1 {
                    255
                } else if dark {
                    0
                } else {
                    255
                };
            }
        }
        (pixels, width, stride)
    }

    #[derive(Debug, PartialEq)]
    struct Outcome {
        result: Result<Option<Detection>, Error>,
        stats: Stats,
        error: Option<crate::core::Error>,
        hint: Option<([Point; 4], usize)>,
        payload: Vec<u8>,
        segments: Vec<Segment>,
    }

    fn run(
        reference: bool,
        image: GrayImage<'_>,
        side: usize,
        scale: usize,
        strategy: SamplingStrategy,
        prior: crate::core::Error,
        limit: usize,
    ) -> (Outcome, usize) {
        let budget = PixelBudget::new(limit);
        let bin = Binary::<true> {
            image,
            budget: &budget,
            cache: None,
            memo: None,
            fill_cache: false,
            threshold: 127,
            inverted: false,
            local: false,
            range: [0, 255],
            radius: 24,
        };
        let mut matrix = [0xa5; MAX_MATRIX_BYTES];
        let mut core = [0xa5; crate::core::SCRATCH_BYTES];
        let mut output = [0; 7089];
        let mut segments = [Segment::EMPTY; 64];
        let mut stats = Stats::default();
        let mut hint = None;
        let mut search = SearchContext::<true> {
            matrix: &mut matrix,
            core_scratch: &mut core,
            output: &mut output,
            segments: &mut segments,
            stats: &mut stats,
            max_candidates: 4096,
            max_retries: 2,
            last_error: Some(prior),
            strategy,
            candidate_hint: Some(&mut hint),
        };
        let h = Homography([scale as f32, 0., 0., 0., scale as f32, 0., 0., 0.]);
        let result = if reference {
            search.attempt_reference(bin, h, side)
        } else {
            search.attempt(bin, h, side)
        };
        let error = search.last_error;
        let (payload_len, segment_len) = result
            .as_ref()
            .ok()
            .and_then(|v| v.as_ref())
            .map_or((0, 0), |d| (d.decoded.payload_len, d.decoded.segment_count));
        assert!(budget.used.get() <= limit);
        (
            Outcome {
                result,
                stats,
                error,
                hint,
                payload: output[..payload_len].to_vec(),
                segments: segments[..segment_len].to_vec(),
            },
            budget.used.get(),
        )
    }

    #[test]
    fn header_reserve_gate_covers_exact_boundaries_after_center_reads() {
        for side in [21usize, 45] {
            let header_modules = if side >= 45 { 67 } else { 31 };
            let valid = if side == 21 {
                include_bytes!("../../../fixtures/v1.bin").as_slice()
            } else {
                include_bytes!("../../../fixtures/v7.bin").as_slice()
            };
            let failed = if side == 21 {
                std::vec![0; (side*side).div_ceil(8)]
            } else {
                wrong_version_bits()
            };
            let failure = if side == 21 {
                crate::core::Error::Format
            } else {
                crate::core::Error::Version
            };
            for strategy in [
                SamplingStrategy::Center,
                SamplingStrategy::CenterThenMajority5,
            ] {
                let majority = strategy == SamplingStrategy::CenterThenMajority5;
                let center_reads = if majority { side * side } else { 0 };
                let sample_reads = if majority { 5 } else { 1 };
                let gate = center_reads + (side * side + header_modules) * sample_reads;
                for successful in [false, true] {
                    let bits = if successful { valid } else { failed.as_slice() };
                    // Whiten just each module's center: center sampling fails
                    // Format, but the four surrounding votes recover valid QR.
                    let scale = if successful && majority { 3 } else { 1 };
                    let (pixels, width, stride) = render(bits, side, scale, successful && majority);
                    let image = GrayImage::new(&pixels, width, width, stride).unwrap();
                    for limit in [gate - 1, gate, gate + 1] {
                        let prior = if successful {
                            crate::core::Error::Format
                        } else {
                            failure
                        };
                        let (expected, _) = run(true, image, side, scale, strategy, prior, limit);
                        let (actual, reads) =
                            run(false, image, side, scale, strategy, prior, limit);
                        assert_eq!(
                            actual, expected,
                            "side={side} majority={majority} success={successful} limit={limit}"
                        );
                        assert_eq!(actual.result.as_ref().unwrap().is_some(), successful);
                        let expected_reads = if limit < gate {
                            center_reads + side * side * sample_reads
                        } else if successful {
                            gate
                        } else {
                            center_reads + header_modules * sample_reads
                        };
                        assert_eq!(
                            reads, expected_reads,
                            "side={side} majority={majority} success={successful} limit={limit}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod range_extrema_tests {
    extern crate std;
    use super::*;
    #[test]
    fn range_early_exit_matches_full_roi_and_charges_only_visited_rows() {
        for width in [1usize, 7, 16, 31, 32, 33, 257] {
            for height in [1usize, 3, 9] {
                let stride = width + 5;
                for kind in 0..5 {
                    let mut pixels = std::vec![0; height * stride];
                    for y in 0..height {
                        for x in 0..width {
                            pixels[y * stride + x] = match kind {
                                0 => 0,
                                1 => 255,
                                2 => 127,
                                3 => ((y * width + x) * 37 % 256) as u8,
                                _ => {
                                    if y == height / 2 && x == 0 {
                                        0
                                    } else {
                                        255
                                    }
                                }
                            };
                        }
                    }
                    let image = GrayImage::new(&pixels, width, height, stride).unwrap();
                    let (range, reads) = complete_range(image);
                    let mut min = 255;
                    let mut max = 0;
                    let mut expected_rows = height;
                    for y in 0..height {
                        for x in 0..width {
                            let value = pixels[y * stride + x];
                            min = min.min(value);
                            max = max.max(value);
                        }
                        if min == 0 && max == 255 && expected_rows == height {
                            expected_rows = y + 1;
                        }
                    }
                    assert_eq!(range, [min, max]);
                    assert_eq!(reads, expected_rows * width);
                    assert!(reads <= width * height);
                }
            }
        }
    }
}

#[cfg(test)]
mod range_budget_review_tests {
    extern crate std;
    use super::*;

    fn check<const OPT: bool>(already_used: usize, remaining: usize) {
        // A strided 2x2 ROI reaches [0,255] in its first row. There are four
        // visible pixels, but optimized range discovery needs only two reads.
        let pixels = [0, 255, 0xa5, 255, 255];
        let image = GrayImage::new(&pixels, 2, 2, 3).unwrap();
        let limit = already_used + remaining;
        let config = Config::<2, 2> {
            row_step: 2, // one row in each finder pass, simplifying exact counts
            max_pixel_reads: limit,
            ..Default::default()
        };
        let mut detector = std::vec![0xa5; config.scratch_bytes().unwrap()];
        let mut core = [0xa5; crate::core::SCRATCH_BYTES];
        let mut output = [0; 32];
        let mut segments = [Segment::EMPTY; 8];
        let mut stats = Stats::default();
        let budget = PixelBudget::new(limit);
        // Simulate work already consumed by a preceding ROI in the same call.
        budget.used.set(already_used);
        let mut state = RegionState::default();
        let result = decode_inner::<2, 2, OPT>(
            image,
            &config,
            SamplingStrategy::Center,
            &mut detector,
            &mut core,
            &mut output,
            &mut segments,
            &mut stats,
            &budget,
            None,
            None,
            false,
            &mut state,
        );
        assert_eq!(result, Err(Error::PixelBudget));
        assert_eq!(budget.used.get(), limit);
        assert!(budget.used.get() >= already_used);
        assert_eq!(stats.candidates, 0);
        assert_eq!(stats.retries, 0);
        if remaining < 4 {
            // The full ROI would not fit: preserve scalar partial failure,
            // even though the first two pixels already contain both extrema.
            assert_eq!(state.range, None);
            assert_eq!(stats.rows, 0);
            assert_eq!(stats.passes, 0);
        } else {
            assert_eq!(state.range, Some([0, 255]));
            // Optimized range discovery saves two reads, finishes the first
            // two-pixel scan row, then stops during the inverted pass. Legacy
            // range discovery spends all four reads and stops in the first pass.
            let passes = if OPT { 2 } else { 1 };
            assert_eq!(stats.rows, passes);
            assert_eq!(stats.passes, passes);
        }
        // A terminal budget remains closed; it is neither reset nor overdrawn.
        assert_eq!(budget.read(image, 0, 0), Err(Error::PixelBudget));
        assert_eq!(budget.used.get(), limit);
    }

    #[test]
    fn nonzero_prior_usage_survives_area_minus_one_exact_and_plus_one() {
        for used in [1usize, 7, 4096] {
            for remaining in [3usize, 4, 5] {
                check::<true>(used, remaining);
                check::<false>(used, remaining);
            }
        }
    }
}

#[cfg(test)]
mod alignment_global_tests {
    extern crate std;
    use super::*;
    fn reference_alignment<const OPT: bool>(
        bin: Binary<'_, OPT>,
        prediction: Point,
        u: Point,
        v: Point,
    ) -> Result<Option<Point>, Error> {
        let mut best = None;
        let mut best_score = -1.0f32;
        // Search up to eight modules around the affine prediction, half-module increments.
        for iy in -16..=16 {
            for ix in -16..=16 {
                let center = prediction
                    .add(u.scale(ix as f32 * 0.5))
                    .add(v.scale(iy as f32 * 0.5));
                let penalty = (ix * ix + iy * iy) as f32 * 0.003;
                if OPT && 25.0 - penalty <= best_score {
                    continue;
                }
                let mut matches = 0;
                let mut valid = true;
                let mut examined = 0;
                'pattern: for y in -2i32..=2 {
                    for x in -2i32..=2 {
                        let p = center.add(u.scale(x as f32)).add(v.scale(y as f32));
                        let expected = x.abs() == 2 || y.abs() == 2 || (x == 0 && y == 0);
                        if let Some(d) = bin.point(p)? {
                            matches += usize::from(d == expected);
                        } else {
                            valid = false;
                        }
                        examined += 1;
                        // At least 23 of 25 points must match. Preserve traversal
                        // and strict score ordering, including ties, while dropping
                        // candidates that cannot become the selected result.
                        if OPT
                            && (!valid
                                || examined - matches > 2
                                || (matches + 25 - examined) as f32 - penalty <= best_score)
                        {
                            valid = false;
                            break 'pattern;
                        }
                    }
                }
                if valid && matches >= 23 {
                    let score = matches as f32 - penalty;
                    if score > best_score {
                        best_score = score;
                        best = Some(center);
                    }
                }
            }
        }
        Ok(best)
    }

    #[test]
    fn integer_alignment_pruning_preserves_results_and_exact_input_reads() {
        let (width, height, stride) = (83, 79, 97);
        let mut pixels: std::vec::Vec<u8> = (0..height * stride)
            .map(|i| ((i * 37 + i / stride * 53) % 256) as u8)
            .collect();
        for (cx, cy) in [(31, 32), (44, 45)] {
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    pixels[(cy + dy) as usize * stride + (cx + dx) as usize] =
                        if dx.abs() == 2 || dy.abs() == 2 || (dx == 0 && dy == 0) {
                            0
                        } else {
                            255
                        };
                }
            }
        }
        let image = GrayImage::new(&pixels, width, height, stride).unwrap();
        for (u, v) in [
            (Point { x: 1.0, y: 0.0 }, Point { x: 0.0, y: 1.0 }),
            (Point { x: 2.3, y: 0.4 }, Point { x: -0.3, y: 1.7 }),
        ] {
            for prediction in [
                Point { x: 0.0, y: 1.0 },
                Point { x: 37.0, y: 38.0 },
                Point { x: 82.0, y: 78.0 },
            ] {
                for inverted in [false, true] {
                    for limit in [0, 1, 50, 1000, usize::MAX] {
                        let old_budget = PixelBudget::new(limit);
                        let new_budget = PixelBudget::new(limit);
                        let old = Binary::<true> {
                            image,
                            budget: &old_budget,
                            cache: None,
                            memo: None,
                            fill_cache: false,
                            threshold: 127,
                            inverted,
                            local: false,
                            range: [0, 255],
                            radius: 24,
                        };
                        let new = Binary {
                            budget: &new_budget,
                            ..old
                        };
                        assert_eq!(
                            find_alignment(new, prediction, u, v),
                            reference_alignment(old, prediction, u, v)
                        );
                        assert_eq!(new_budget.used.get(), old_budget.used.get());
                    }
                }
            }
        }
    }
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod next_tests {
    extern crate std;
    use super::*;
    #[test]
    fn packed_and_generic_cross_preserve_budget_and_results() {
        let mut conditions = 0;
        let mut successes = 0;
        for inverted in [false, true] {
            for radius in [4, 5, 24, 25, 128] {
                let pixels: std::vec::Vec<_> = (0..131 * 9)
                    .map(|i| {
                        let y = i / 9;
                        let dark =
                            (55..58).contains(&y) || (61..70).contains(&y) || (73..76).contains(&y);
                        let v = if dark { 60 } else { 220 };
                        if inverted {
                            255 - v
                        } else {
                            v
                        }
                    })
                    .collect();
                let image = GrayImage::new(&pixels, 9, 131, 9).unwrap();
                let reference_budget = PixelBudget::new(usize::MAX);
                let reference = Binary::<false> {
                    image,
                    budget: &reference_budget,
                    cache: None,
                    memo: None,
                    fill_cache: false,
                    threshold: 127,
                    inverted,
                    local: true,
                    range: [35, 220],
                    radius,
                };
                let expected =
                    cross_check_pixels(image.height, 65, 3.0, |p| reference.dark(4, p)).unwrap();
                successes += usize::from(expected.is_some());
                for packed in [false, true] {
                    for has_memo in [false, true] {
                        for limit in 0..=512 {
                            let budget = PixelBudget::new(limit);
                            let memo: std::vec::Vec<_> =
                                (0..(9 * 131 + 3) / 4).map(|_| Cell::new(0u8)).collect();
                            let bin = Binary::<true> {
                                image,
                                budget: &budget,
                                cache: None,
                                memo: if has_memo { Some(&memo) } else { None },
                                fill_cache: false,
                                threshold: 127,
                                inverted,
                                local: true,
                                range: [35, 220],
                                radius,
                            };
                            let mut bytes = [0xa5; 386];
                            let result = if packed {
                                cross_check_vertical_packed(bin, 4, 65, 3.0, &mut bytes[1..385])
                            } else {
                                cross_check(bin, 4, 65, false, 3.0)
                            };
                            match result {
                                Err(Error::PixelBudget) => assert_eq!(budget.used.get(), limit),
                                Ok(value) => {
                                    assert_eq!(value, expected);
                                    assert!(budget.used.get() <= limit);
                                }
                                other => panic!("{other:?}"),
                            }
                            assert_eq!(bytes[0], 0xa5);
                            assert_eq!(bytes[385], 0xa5);
                            conditions += 1;
                        }
                    }
                }
            }
        }
        assert!(successes > 0);
        std::println!("packed/generic/memo finite conditions={conditions}");
    }
}
