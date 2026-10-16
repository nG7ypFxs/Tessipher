#[cfg(tessipher_repository_fixtures)]
mod regression_tests {
    extern crate std;
    use super::super::*;
    use crate::core::MAX_MATRIX_BYTES;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Error, GrayImage, Point, Rect, Stats, FINDER_BYTES};
    use core::cell::Cell;

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

    #[test]
    fn all_finder_and_alignment_helpers_propagate_exhaustion() {
        let image = GrayImage::new(&[0; 64 * 64], 64, 64, 64).unwrap();
        let center = Point { x: 32.0, y: 32.0 };
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
            assert_eq!(observation_pattern(bin, f), Err(Error::PixelBudget));
            assert_eq!(budget.used.get(), 1);
        }
    }
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod next_tests {
    extern crate std;
    use super::super::*;
    use crate::detect::threshold::{Binary, PixelBudget};
    use crate::detect::{Error, GrayImage};
    use core::cell::Cell;
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
