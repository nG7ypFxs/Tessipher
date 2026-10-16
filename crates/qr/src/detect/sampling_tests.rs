#[cfg(tessipher_repository_fixtures)]
mod regression_tests {
    extern crate std;
    use super::super::*;
    use crate::core::MAX_MATRIX_BYTES;
    use crate::detect::scan::Finder;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::tracking::track_hint;
    use crate::detect::{
        Error, GrayImage, Point, SamplingStrategy, Stats, DEFAULT_MAX_PIXEL_READS,
    };

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
    fn sampling_pattern_and_alignment_helpers_propagate_exhaustion() {
        let image = GrayImage::new(&[0; 64 * 64], 64, 64, 64).unwrap();
        let center = Point { x: 32.0, y: 32.0 };
        let u = Point { x: 1.0, y: 0.0 };
        let v = Point { x: 0.0, y: 1.0 };
        for local in [false, true] {
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(finder_pattern(bin, center, u, v), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
            let budget = PixelBudget::new(1);
            let bin = test_binary(image, &budget, local);
            assert_eq!(find_alignment(bin, center, u, v), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
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
        let bits = include_bytes!("../../../../fixtures/v2.bin");
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
}

#[cfg(test)]
mod sampling_specialization_tests {
    extern crate std;
    use super::super::*;
    use crate::core::MAX_MATRIX_BYTES;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Error, GrayImage, SamplingStrategy, Stats};
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

#[cfg(all(test, tessipher_repository_fixtures))]
mod header_prefilter_tests {
    extern crate std;
    use super::super::*;
    use crate::core::{Segment, MAX_MATRIX_BYTES};
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Detection, Error, GrayImage, Point, SamplingStrategy, Stats};
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
            (21, include_bytes!("../../../../fixtures/v1.bin").as_slice()),
            (45, include_bytes!("../../../../fixtures/v7.bin").as_slice()),
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
    use super::super::*;
    use crate::core::{Segment, MAX_MATRIX_BYTES};
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Detection, Error, GrayImage, Point, SamplingStrategy, Stats};
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
        let mut bits = include_bytes!("../../../../fixtures/v7.bin").to_vec();
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
        let valid = include_bytes!("../../../../fixtures/v7.bin");
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
                include_bytes!("../../../../fixtures/v1.bin").as_slice()
            } else {
                include_bytes!("../../../../fixtures/v7.bin").as_slice()
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
mod alignment_global_tests {
    extern crate std;
    use super::super::*;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Error, GrayImage, Point};
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
