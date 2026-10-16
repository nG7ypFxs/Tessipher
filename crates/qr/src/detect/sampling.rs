//! Candidate geometry, projection sampling, and core decode attempts.

use super::scan::Finder;
use super::threshold::Binary;
use super::tracking::track_hint;
use super::{Detection, Error, Point, SamplingStrategy, Segment, Stats};
use crate::core::Matrix;

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
pub(super) struct Homography(pub(super) [f32; 8]);
impl Homography {
    pub(super) fn at(self, p: Point) -> Point {
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
pub(super) struct SearchContext<'a, const OPT: bool = false> {
    pub(super) matrix: &'a mut [u8],
    pub(super) core_scratch: &'a mut [u8],
    pub(super) output: &'a mut [u8],
    pub(super) segments: &'a mut [Segment],
    pub(super) stats: &'a mut Stats,
    pub(super) max_candidates: usize,
    pub(super) max_retries: usize,
    pub(super) last_error: Option<crate::core::Error>,
    pub(super) strategy: SamplingStrategy,
    pub(super) candidate_hint: Option<&'a mut Option<([Point; 4], usize)>>,
}
impl<const OPT: bool> SearchContext<'_, OPT> {
    pub(super) fn candidate(
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

pub(super) fn projection_corners(h: Homography, side: usize) -> [Point; 4] {
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

pub(super) fn finder_pattern<const OPT: bool>(
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

#[cfg(test)]
#[path = "sampling_tests.rs"]
mod tests;
