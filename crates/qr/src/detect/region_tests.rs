#[cfg(tessipher_repository_fixtures)]
mod regression_tests {
    extern crate std;
    use crate::core::MAX_MATRIX_BYTES;
    use crate::detect::{
        decode_with_mode, Config, DetectionMode, Error, GrayImage, SamplingStrategy, Stats,
        FINDER_BYTES,
    };

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
}

#[cfg(test)]
mod region_reuse_tests {
    extern crate std;
    use super::super::*;
    use crate::core::Segment;
    use crate::detect::threshold::PixelBudget;
    use crate::detect::{
        Config, DefaultConfig, Detection, DetectionMode, Error, GrayImage, Point, SamplingStrategy,
        Stats,
    };
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
mod fast_refinement_policy_tests {
    use super::super::*;
    use crate::core::MAX_MATRIX_BYTES;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{
        decode_with_mode, Config, DetectionMode, Error, GrayImage, SamplingStrategy, Stats,
        FINDER_BYTES,
    };

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

#[cfg(test)]
mod range_extrema_tests {
    extern crate std;
    use super::super::*;
    use crate::detect::GrayImage;
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
    use super::super::*;
    use crate::core::Segment;
    use crate::detect::threshold::PixelBudget;
    use crate::detect::{Config, Error, GrayImage, SamplingStrategy, Stats};

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
