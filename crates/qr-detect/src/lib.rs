//! Bounded Gray8 detector. No frame-sized binary image, allocation, or copied input.
//! Global midrange and local sampled-mean thresholds, each in normal/inverted polarity.
#![no_std]
#![forbid(unsafe_code)]
use qr_core::{Decoded, Matrix, Segment, MAX_MATRIX_BYTES};

pub const SCRATCH_ALIGN: usize = 1;
const FINDER_BYTES: usize = 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidImage,
    ImageTooLarge,
    InvalidConfig,
    ScratchTooSmall,
    FinderCapacity,
    CandidateBudget,
    RetryBudget,
    NotFound,
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
    pub fn width(self) -> usize {
        self.width
    }
    pub fn height(self) -> usize {
        self.height
    }
    pub fn stride(self) -> usize {
        self.stride
    }
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
/// Image maxima are independent const parameters. Changing them does not change scratch size.
#[derive(Clone, Copy, Debug)]
pub struct Config<const MAX_WIDTH: usize = 640, const MAX_HEIGHT: usize = 640> {
    pub max_finders: usize,
    /// Total triples examined across all threshold/polarity passes.
    pub max_candidates: usize,
    /// Total sampled matrices (including rejected geometric samples) across all passes.
    pub max_retries: usize,
    /// Row scan increment. One preserves the best small-symbol recall.
    pub row_step: usize,
    /// Half-width of the sampled local-threshold window, in pixels (4..=128).
    pub local_radius: usize,
}
impl<const W: usize, const H: usize> Default for Config<W, H> {
    fn default() -> Self {
        Self {
            max_finders: 32,
            max_candidates: 4096,
            max_retries: 256,
            row_step: 1,
            local_radius: 24,
        }
    }
}
impl<const W: usize, const H: usize> Config<W, H> {
    pub const MAX_WIDTH: usize = W;
    pub const MAX_HEIGHT: usize = H;
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
pub type DefaultConfig = Config<640, 640>;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Point {
    pub x: f32,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub rows: usize,
    pub finder_observations: usize,
    pub candidates: usize,
    pub retries: usize,
    pub passes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    pub decoded: Decoded,
    pub corners: [Point; 4],
    pub inverted: bool,
    pub local_threshold: bool,
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
#[derive(Clone, Copy)]
struct Binary<'a> {
    image: GrayImage<'a>,
    threshold: u8,
    inverted: bool,
    local: bool,
    radius: usize,
}
impl Binary<'_> {
    fn dark(self, x: usize, y: usize) -> bool {
        let pixel = self.image.at(x, y) as u32;
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
                    let v = self.image.at(px, py);
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
        dark ^ self.inverted
    }
    fn point(self, p: Point) -> Option<bool> {
        if !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.0
            || p.y < 0.0
            || p.x >= self.image.width as f32
            || p.y >= self.image.height as f32
        {
            None
        } else {
            Some(self.dark(p.x as usize, p.y as usize))
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
) -> Option<(f32, f32)> {
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
    if !get(center) {
        return None;
    }
    let mut r = [0usize; 5];
    let bound = (expected * 6.0 + 5.0) as usize;
    let mut low = center as isize;
    while low >= 0 && get(low as usize) && r[2] <= bound {
        r[2] += 1;
        low -= 1;
    }
    while low >= 0 && !get(low as usize) && r[1] <= bound {
        r[1] += 1;
        low -= 1;
    }
    while low >= 0 && get(low as usize) && r[0] <= bound {
        r[0] += 1;
        low -= 1;
    }
    let mut high = center + 1;
    while high < limit && get(high) && r[2] <= bound {
        r[2] += 1;
        high += 1;
    }
    let center_high = high;
    while high < limit && !get(high) && r[3] <= bound {
        r[3] += 1;
        high += 1;
    }
    while high < limit && get(high) && r[4] <= bound {
        r[4] += 1;
        high += 1;
    }
    let module = ratio(&r)?;
    if module < expected * 0.4 || module > expected * 2.5 {
        return None;
    }
    Some((center_high as f32 - r[2] as f32 * 0.5, module))
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
) -> (usize, bool) {
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        let mut runs = [0usize; 5];
        let mut color = false;
        for x in 0..=bin.image.width {
            let next = x < bin.image.width && bin.dark(x, y);
            if next == color {
                runs[4] += 1;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        if let Some((cy, my)) = cross_check(bin, cx as usize, y, false, module) {
                            if let Some((rx, mx)) =
                                cross_check(bin, cx as usize, cy as usize, true, my)
                            {
                                stats.finder_observations += 1;
                                let f = Finder {
                                    p: Point { x: rx, y: cy },
                                    pitch_x: mx,
                                    pitch_y: my,
                                    hits: 1,
                                };
                                if observation_pattern(bin, f)
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
    (count, overflow)
}
/// Decodes at most one symbol, using deterministic threshold/polarity/triple/version priority.
/// All budgets are shared across the complete call. See `docs/detection.md` for the search envelope.
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
/// Like `decode`, retaining work counters on both success and failure.
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
            let v = image.at(x, y);
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
        );
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
fn find_alignment(bin: Binary<'_>, prediction: Point, u: Point, v: Point) -> Option<Point> {
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
                    if let Some(d) = bin.point(p) {
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
    best
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
        if !finder_pattern(bin, tl.p, u, v)
            || !finder_pattern(bin, tr.p, u, v)
            || !finder_pattern(bin, bl.p, u, v)
        {
            return Ok(None);
        }
        let anchor = side as f32 - 6.5;
        let predicted = tl.p.add(u.add(v).scale(anchor - 3.5));
        let aligned = if version > 1 {
            find_alignment(bin, predicted, u, v)
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
                })) else {
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

fn finder_pattern(bin: Binary<'_>, center: Point, u: Point, v: Point) -> bool {
    let mut good = 0;
    for y in -3i32..=3 {
        for x in -3i32..=3 {
            let edge = x.abs().max(y.abs());
            let expected = edge == 3 || edge <= 1;
            let p = center.add(u.scale(x as f32)).add(v.scale(y as f32));
            if bin.point(p) == Some(expected) {
                good += 1;
            }
        }
    }
    good >= 41
}

fn observation_pattern(bin: Binary<'_>, f: Finder) -> bool {
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
                if bin.point(p) == Some(expected) {
                    good += 1;
                }
            }
        }
        if good >= 37 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_geometric_sample_consumes_retry_and_preserves_decode_error() {
        let image = GrayImage::new(&[255; 21 * 21], 21, 21, 21).unwrap();
        let bin = Binary {
            image,
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
}
