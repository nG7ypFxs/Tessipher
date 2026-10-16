//! Feature-gated single-finder Micro QR image detection.

use super::sampling::{finder_pattern, projection_corners, Homography};
use super::scan::{read_finder, scan, Finder};
use super::threshold::{Binary, PixelBudget};
use super::{
    validate_input, Config, Error, GrayImage, MicroDetection, Point, SamplingStrategy, Segment,
    Stats,
};
use crate::core::{MicroMatrix, MAX_MATRIX_BYTES};

/// Detects and decodes one Micro QR symbol using its single finder pattern.
///
/// This opt-in API shares [`Config`] validation, caller-owned scratch, pass
/// ordering, polarity handling, and work budgets with Model 2 detection. Supply
/// at least [`crate::core::MICRO_SCRATCH_BYTES`] bytes of core scratch.
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
