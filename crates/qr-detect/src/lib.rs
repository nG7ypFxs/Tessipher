//! Bounded Gray8 detector. No frame-sized binary image, allocation, or copied input.
//! Global midrange and local sampled-mean thresholds, each in normal/inverted polarity.
//!
//! [`GrayImage`] borrows a rectangular ROI, including a parent frame's row stride.
//! [`Config`] sets independent image maxima and finder/candidate/sampling and pixel-read budgets.
//! [`decode`] returns the first decoded symbol. [`decode_with_stats`] also exposes
//! counters after failure. No call enumerates every symbol in an image.
//!
//! ```
//! use qr_detect::{Config, GrayImage};
//! let pixels = [255; 8];
//! let roi = GrayImage::new(&pixels, 2, 2, 6)?;
//! assert_eq!(roi.pixel(1, 1), Some(255));
//! let config = Config::<640, 240>::default();
//! assert_eq!(config.scratch_bytes()?, 4557);
//! # Ok::<(), qr_detect::Error>(())
//! ```
#![no_std]
#![forbid(unsafe_code)]
use core::cell::Cell;
use qr_core::{Decoded, Matrix, Segment, MAX_MATRIX_BYTES};

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
    Decode(qr_core::Error),
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
    fn at(self, x: usize, y: usize) -> u8 {
        self.data[y * self.stride + x]
    }
}
/// Default maximum actual input pixel reads across a complete decode call.
/// Independent of image dimensions; includes min/max, repeated and local reads.
pub const DEFAULT_MAX_PIXEL_READS: usize = 64_000_000;

/// Image maxima are independent const parameters. Changing them does not change scratch size.
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
    /// Each local threshold decision costs 26 reads, even at clamped edges.
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
    /// Returns `qr_core::MAX_MATRIX_BYTES + 20 * max_finders` after validation.
    ///
    /// Returns [`Error::InvalidConfig`] for any invalid setting or image maximum.
    /// The returned allocation is byte-aligned and may be reused between calls.
    pub fn scratch_bytes(&self) -> Result<usize, Error> {
        self.validate()?;
        Ok(MAX_MATRIX_BYTES + self.max_finders * FINDER_BYTES)
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
    /// Actual input pixel reads, including repeats; never exceeds max_pixel_reads.
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
    /// Threshold/polarity passes entered (0..=4).
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
#[derive(Clone, Copy)]
struct Binary<'a> {
    image: GrayImage<'a>,
    budget: &'a PixelBudget,
    threshold: u8,
    inverted: bool,
    local: bool,
    radius: usize,
}
impl Binary<'_> {
    fn dark(self, x: usize, y: usize) -> Result<bool, Error> {
        let pixel = self.budget.read(self.image, x, y)? as u32;
        let dark = if !self.local {
            pixel <= self.threshold as u32
        } else {
            // A fixed 5x5 sample window: bounded work, no full-frame or line integral buffer.
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
                    let v = self.budget.read(self.image, px, py)?;
                    sum += v as u32;
                    min = min.min(v);
                    max = max.max(v);
                }
            }
            if max - min < 12 {
                pixel <= self.threshold as u32
            } else {
                pixel * 25 < sum
            }
        };
        Ok(dark ^ self.inverted)
    }
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
fn cross_check(
    bin: Binary<'_>,
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
fn scan(
    bin: Binary<'_>,
    storage: &mut [u8],
    cap: usize,
    step: usize,
    stats: &mut Stats,
) -> Result<(usize, bool), Error> {
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        let mut runs = [0usize; 5];
        let mut color = false;
        for x in 0..=bin.image.width {
            let next = x < bin.image.width && bin.dark(x, y)?;
            if next == color {
                runs[4] += 1;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        if let Some((cy, my)) = cross_check(bin, cx as usize, y, false, module)? {
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
/// Decodes at most one symbol, using deterministic threshold/polarity/triple/version priority.
/// All budgets are shared across the complete call. Thresholds are tried in order:
/// global normal, global inverted, local normal, local inverted.
///
/// Supply detector scratch from [`Config::scratch_bytes`] and core scratch of at
/// least [`qr_core::SCRATCH_BYTES`] bytes. All writable storage belongs to the caller
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
    *stats = Stats::default();
    let budget = PixelBudget::new(config.max_pixel_reads);
    let result = decode_inner(
        image,
        config,
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
#[allow(clippy::too_many_arguments)]
fn decode_inner<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
) -> Result<Detection, Error> {
    let needed = config.scratch_bytes()?;
    if image.width > W || image.height > H {
        return Err(Error::ImageTooLarge);
    }
    if scratch.len() < needed {
        return Err(Error::ScratchTooSmall);
    }
    if core_scratch.len() < qr_core::SCRATCH_BYTES {
        return Err(Error::Decode(qr_core::Error::ScratchTooSmall));
    }
    let (matrix, finders) = scratch.split_at_mut(MAX_MATRIX_BYTES);
    let mut min = 255u8;
    let mut max = 0u8;
    for y in 0..image.height {
        for x in 0..image.width {
            let v = budget.read(image, x, y)?;
            min = min.min(v);
            max = max.max(v);
        }
    }
    if max - min < 8 {
        return Err(Error::NotFound);
    }
    let threshold = ((min as u16 + max as u16) / 2) as u8;
    let mut overflow = false;
    let mut search = SearchContext {
        matrix,
        core_scratch,
        output,
        segments,
        stats,
        max_candidates: config.max_candidates,
        max_retries: config.max_retries,
        last_error: None,
    };
    for pass in 0..4 {
        search.stats.passes += 1;
        let bin = Binary {
            image,
            budget,
            threshold,
            inverted: pass % 2 != 0,
            local: pass >= 2,
            radius: config.local_radius,
        };
        let (count, full) = scan(
            bin,
            finders,
            config.max_finders,
            config.row_step,
            search.stats,
        )?;
        overflow |= full;
        for a in 0..count {
            for b in a + 1..count {
                for c in b + 1..count {
                    let set = [
                        read_finder(finders, a),
                        read_finder(finders, b),
                        read_finder(finders, c),
                    ];
                    if let Some(found) = search.candidate(bin, set)? {
                        return Ok(found);
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
fn find_alignment(
    bin: Binary<'_>,
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
            let mut matches = 0;
            let mut valid = true;
            for y in -2i32..=2 {
                for x in -2i32..=2 {
                    let p = center.add(u.scale(x as f32)).add(v.scale(y as f32));
                    let expected = x.abs() == 2 || y.abs() == 2 || (x == 0 && y == 0);
                    if let Some(d) = bin.point(p)? {
                        matches += usize::from(d == expected);
                    } else {
                        valid = false;
                    }
                }
            }
            if valid && matches >= 23 {
                let score = matches as f32 - (ix * ix + iy * iy) as f32 * 0.003;
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
struct SearchContext<'a> {
    matrix: &'a mut [u8],
    core_scratch: &'a mut [u8],
    output: &'a mut [u8],
    segments: &'a mut [Segment],
    stats: &'a mut Stats,
    max_candidates: usize,
    max_retries: usize,
    last_error: Option<qr_core::Error>,
}
impl SearchContext<'_> {
    fn candidate(&mut self, bin: Binary<'_>, set: [Finder; 3]) -> Result<Option<Detection>, Error> {
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
        bin: Binary<'_>,
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
        bin: Binary<'_>,
        h: Homography,
        side: usize,
    ) -> Result<Option<Detection>, Error> {
        if self.stats.retries >= self.max_retries {
            return Err(Error::RetryBudget);
        }
        self.stats.retries += 1;
        self.matrix[..(side * side).div_ceil(8)].fill(0);
        for y in 0..side {
            for x in 0..side {
                let Some(d) = bin.point(h.at(Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                }))?
                else {
                    return Ok(None);
                };
                if d {
                    let i = y * side + x;
                    self.matrix[i / 8] |= 128 >> (i % 8);
                }
            }
        }
        let m = Matrix::new(self.matrix, side).map_err(Error::Decode)?;
        match qr_core::decode(m, self.core_scratch, self.output, self.segments) {
            Ok(decoded) => Ok(Some(Detection {
                decoded,
                corners: [
                    h.at(Point { x: 0.0, y: 0.0 }),
                    h.at(Point {
                        x: side as f32,
                        y: 0.0,
                    }),
                    h.at(Point {
                        x: side as f32,
                        y: side as f32,
                    }),
                    h.at(Point {
                        x: 0.0,
                        y: side as f32,
                    }),
                ],
                inverted: bin.inverted,
                local_threshold: bin.local,
                stats: *self.stats,
            })),
            Err(
                e @ (qr_core::Error::OutputTooSmall
                | qr_core::Error::SegmentsTooSmall
                | qr_core::Error::ScratchTooSmall),
            ) => Err(Error::Decode(e)),
            Err(e) => {
                self.last_error = Some(e);
                Ok(None)
            }
        }
    }
}

fn finder_pattern(bin: Binary<'_>, center: Point, u: Point, v: Point) -> Result<bool, Error> {
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

fn observation_pattern(bin: Binary<'_>, f: Finder) -> Result<bool, Error> {
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
                if bin.point(p)? == Some(expected) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_geometric_sample_consumes_retry_and_preserves_decode_error() {
        let budget = PixelBudget::new(DEFAULT_MAX_PIXEL_READS);
        let image = GrayImage::new(&[255; 21 * 21], 21, 21, 21).unwrap();
        let bin = Binary {
            image,
            budget: &budget,
            threshold: 127,
            inverted: false,
            local: false,
            radius: 24,
        };
        let mut matrix = [0; MAX_MATRIX_BYTES];
        let mut core_scratch = [0; qr_core::SCRATCH_BYTES];
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
            last_error: Some(qr_core::Error::Version),
        };
        // The first sampled point lies outside the image.
        let outside = Homography([1.0, 0.0, -10.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert_eq!(search.attempt(bin, outside, 21), Ok(None));
        assert_eq!(search.stats.retries, 2);
        assert_eq!(search.last_error, Some(qr_core::Error::Version));
        assert_eq!(search.attempt(bin, outside, 21), Err(Error::RetryBudget));
        assert_eq!(search.stats.retries, 2);
    }
    fn test_binary<'a>(image: GrayImage<'a>, budget: &'a PixelBudget, local: bool) -> Binary<'a> {
        Binary {
            image,
            budget,
            threshold: 127,
            inverted: false,
            local,
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
            let mut cs = [0; qr_core::SCRATCH_BYTES];
            let mut stats = Stats::default();
            let mut search = SearchContext {
                matrix: &mut matrix,
                core_scratch: &mut cs,
                output: &mut [],
                segments: &mut [],
                stats: &mut stats,
                max_candidates: 1,
                max_retries: 2,
                last_error: Some(qr_core::Error::Version),
            };
            assert_eq!(search.attempt(bin, h, 21), Err(Error::PixelBudget));
            assert_eq!(search.last_error, Some(qr_core::Error::Version));
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
        let mut cs = [0; qr_core::SCRATCH_BYTES];
        let mut stats = Stats::default();
        let mut search = SearchContext {
            matrix: &mut matrix,
            core_scratch: &mut cs,
            output: &mut [],
            segments: &mut [],
            stats: &mut stats,
            max_candidates: 1,
            max_retries: 2,
            last_error: Some(qr_core::Error::Version),
        };
        assert_eq!(search.candidate(bin, finders), Err(Error::PixelBudget));
        assert_eq!(budget.used.get(), 148);
        assert_eq!(search.stats.candidates, 1);
        assert_eq!(search.stats.retries, 0);
        assert_eq!(search.last_error, Some(qr_core::Error::Version));
    }
}
