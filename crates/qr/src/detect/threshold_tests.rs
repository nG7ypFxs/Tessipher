#[cfg(tessipher_repository_fixtures)]
mod regression_tests {
    extern crate std;
    use super::super::*;
    use crate::detect::{Error, GrayImage, Rect};
    use core::cell::{Cell, RefCell};

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
}

#[cfg(all(test, tessipher_repository_fixtures))]
mod memo_tests {
    extern crate std;
    use super::super::*;
    use crate::detect::{DefaultConfig, DetectionMode, Error, GrayImage, Rect};
    use core::cell::Cell;

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
