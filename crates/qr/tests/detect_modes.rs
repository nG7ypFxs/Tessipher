#![cfg(all(feature = "detect", tessipher_repository_fixtures))]

use tessipher::core::{Segment, SCRATCH_BYTES};
use tessipher::detect::*;

const IMAGES: &[&[u8]] = &[
    include_bytes!("../../../fixtures/v1.pgm"),
    include_bytes!("../../../fixtures/v2.pgm"),
    include_bytes!("../../../fixtures/v7.pgm"),
    include_bytes!("../../../fixtures/v14.pgm"),
    include_bytes!("../../../fixtures/v32.pgm"),
    include_bytes!("../../../fixtures/v40.pgm"),
    include_bytes!("../../../fixtures/perspective-v1.pgm"),
    include_bytes!("../../../fixtures/perspective-strong-v1.pgm"),
    include_bytes!("../../../fixtures/perspective-v7.pgm"),
    include_bytes!("../../../fixtures/rotated-v2.pgm"),
    include_bytes!("../../../fixtures/rotated45-v2.pgm"),
    include_bytes!("../../../fixtures/lighting-v2.pgm"),
];
const MODES: [DetectionMode; 2] = [DetectionMode::Direct, DetectionMode::Fast];
const STRATEGIES: [SamplingStrategy; 2] = [
    SamplingStrategy::Center,
    SamplingStrategy::CenterThenMajority5,
];
fn pgm(file: &[u8]) -> GrayImage<'_> {
    let mut lines = file.splitn(4, |&b| b == b'\n');
    assert_eq!(lines.next(), Some(&b"P5"[..]));
    let mut dimensions = core::str::from_utf8(lines.next().unwrap())
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse::<usize>().unwrap());
    let (width, height) = (dimensions.next().unwrap(), dimensions.next().unwrap());
    assert_eq!(lines.next(), Some(&b"255"[..]));
    GrayImage::new(lines.next().unwrap(), width, height, width).unwrap()
}

#[test]
fn all_modes_decode_corpus_with_both_sampling_strategies_and_exact_scratch() {
    assert_eq!(DetectionMode::default(), DetectionMode::Direct);
    let config = DefaultConfig::default();
    let needed = config.scratch_bytes().unwrap();
    let mut ds = vec![0xaa; needed + 13];
    let mut cs = vec![0xbb; SCRATCH_BYTES + 11];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for (i, file) in IMAGES.iter().enumerate() {
        for mode in MODES {
            for strategy in STRATEGIES {
                let mut stats = Stats::default();
                let found = decode_with_mode(
                    pgm(file),
                    &config,
                    mode,
                    strategy,
                    &mut ds,
                    &mut cs[..SCRATCH_BYTES],
                    &mut output,
                    &mut segments,
                    &mut stats,
                )
                .unwrap_or_else(|e| {
                    panic!("image {i}, mode {mode:?}, strategy {strategy:?}: {e:?}")
                });
                assert_eq!(&output[..found.decoded.payload_len], b"TEST");
                assert_eq!(stats, found.stats);
                assert!(stats.pixel_reads <= config.max_pixel_reads);
                assert!(stats.passes <= if mode == DetectionMode::Fast { 8 } else { 4 });
                assert!(ds[needed..].iter().all(|&b| b == 0xaa));
                assert!(cs[SCRATCH_BYTES..].iter().all(|&b| b == 0xbb));
            }
        }
    }
}

#[test]
fn direct_matches_default_entrypoints_including_counters() {
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let mut default_tracker = Tracker::new();
    let mut explicit_tracker = Tracker::new();
    for file in IMAGES {
        for strategy in STRATEGIES {
            let mut default_stats = Stats::default();
            let default = decode_with_sampling(
                pgm(file),
                &config,
                strategy,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut default_stats,
            );
            let mut stats = Stats::default();
            let current = tessipher::decode_gray_with_mode(
                pgm(file),
                &config,
                DetectionMode::Direct,
                strategy,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats,
            );
            assert_eq!(default, current);
            assert_eq!(default_stats, stats);
            if strategy == SamplingStrategy::Center {
                let plain = decode(
                    pgm(file),
                    &config,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                );
                assert_eq!(plain, current);
                let with_stats = decode_with_stats(
                    pgm(file),
                    &config,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut default_stats,
                );
                assert_eq!(with_stats, current);
                assert_eq!(default_stats, stats);
            }
            let default = default_tracker.decode_with_stats(
                pgm(file),
                &config,
                strategy,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut default_stats,
            );
            let explicit = explicit_tracker.decode_with_mode(
                pgm(file),
                &config,
                DetectionMode::Direct,
                strategy,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats,
            );
            assert_eq!(default, explicit);
            assert_eq!(default_stats, stats);
            assert_eq!(default_tracker.state(), explicit_tracker.state());
        }
    }
}

#[test]
fn every_mode_preserves_validation_and_capacity_errors_and_exact_pixel_budgets() {
    let image = pgm(IMAGES[0]);
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for mode in MODES {
        let mut stats = Stats {
            rows: 23,
            ..Stats::default()
        };
        assert_eq!(
            decode_with_mode(
                image,
                &config,
                mode,
                SamplingStrategy::Center,
                &mut [],
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats
            ),
            Err(Error::ScratchTooSmall)
        );
        assert_eq!(stats, Stats::default());
        let invalid = DefaultConfig {
            row_step: 0,
            ..config
        };
        assert_eq!(
            decode_with_mode(
                image,
                &invalid,
                mode,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats
            ),
            Err(Error::InvalidConfig)
        );
        assert_eq!(stats, Stats::default());
        assert_eq!(
            decode_with_mode(
                image,
                &config,
                mode,
                SamplingStrategy::Center,
                &mut ds,
                &mut [],
                &mut output,
                &mut segments,
                &mut stats
            ),
            Err(Error::Decode(tessipher::core::Error::ScratchTooSmall))
        );
        assert_eq!(stats, Stats::default());
        for (empty_output, empty_segments, error) in [
            (true, false, tessipher::core::Error::OutputTooSmall),
            (false, true, tessipher::core::Error::SegmentsTooSmall),
        ] {
            assert_eq!(
                decode_with_mode(
                    image,
                    &config,
                    mode,
                    SamplingStrategy::Center,
                    &mut ds,
                    &mut cs,
                    if empty_output { &mut [] } else { &mut output },
                    if empty_segments {
                        &mut []
                    } else {
                        &mut segments
                    },
                    &mut stats
                ),
                Err(Error::Decode(error))
            );
            assert_eq!(stats.passes, 1);
            assert_eq!(stats.retries, 1);
        }
        let area = image.width() * image.height();
        for max_pixel_reads in [1, 17, area - 1, area + 1] {
            let limited = DefaultConfig {
                max_pixel_reads,
                ..config
            };
            let result = decode_with_mode(
                image,
                &limited,
                mode,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats,
            );
            if mode == DetectionMode::Fast && max_pixel_reads == area + 1 {
                // A full-ROI reservation enables exact 0/255 range completion
                // after 21 rows; the saved reads now fit the successful scan.
                let found = result.unwrap();
                assert_eq!(&output[..found.decoded.payload_len], b"TEST");
                assert!(stats.pixel_reads < max_pixel_reads);
            } else {
                assert_eq!(result, Err(Error::PixelBudget));
                assert_eq!(stats.pixel_reads, max_pixel_reads);
            }
            assert!(stats.passes <= 1);
        }
    }
}

#[test]
fn column_summaries_fit_at_the_boundary_and_wide_images_use_direct_thresholds() {
    let limit = tessipher::core::MAX_MATRIX_BYTES / 4;
    for width in [limit, limit + 1] {
        let pixels: Vec<_> = (0..width * 3)
            .map(|x| if x % 2 == 0 { 0 } else { 255 })
            .collect();
        let image = GrayImage::new(&pixels, width, 3, width).unwrap();
        let config = Config::<2048, 4>::default();
        let mut ds = vec![0xaa; config.scratch_bytes().unwrap() + 3];
        let mut cs = [0; SCRATCH_BYTES];
        let mut stats = Stats::default();
        let result = decode_with_mode(
            image,
            &config,
            DetectionMode::Direct,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut [],
            &mut [],
            &mut stats,
        );
        assert_eq!(result, Err(Error::NotFound));
        if width == limit {
            // Exact 0/255 range discovery reads one row instead of three.
            assert_eq!(stats.pixel_reads, width + width * 3 * 14);
            assert!(ds[width * 4..].iter().all(|&b| b == 0xaa));
        } else {
            assert!(stats.pixel_reads <= width * 3 * 55);
            assert_eq!(stats.rows, 3 * 4);
            assert_eq!(stats.passes, 4);
            assert_eq!(stats.finder_observations, 0);
            assert!(ds.iter().all(|&b| b == 0xaa));
        }
    }
}

#[test]
fn tracker_modes_track_recover_and_keep_state_on_terminal_errors() {
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for mode in MODES {
        let mut tracker = Tracker::new();
        let mut stats = Stats::default();
        for &file in [IMAGES[0], IMAGES[1], IMAGES[0]].iter() {
            let found = tracker
                .decode_with_mode(
                    pgm(file),
                    &config,
                    mode,
                    SamplingStrategy::CenterThenMajority5,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats,
                )
                .unwrap();
            assert_eq!(&output[..found.decoded.payload_len], b"TEST");
            assert_eq!(found.stats, stats);
            assert!(matches!(tracker.state(), TrackerState::Tracking { .. }));
        }
        let previous = tracker.state();
        let limited = DefaultConfig {
            max_pixel_reads: 1,
            ..config
        };
        assert_eq!(
            tracker.decode_with_mode(
                pgm(IMAGES[0]),
                &limited,
                mode,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats
            ),
            Err(Error::PixelBudget)
        );
        assert_eq!(stats.pixel_reads, 1);
        assert_eq!(tracker.state(), previous);
        assert_eq!(
            tracker.decode_with_mode(
                pgm(IMAGES[0]),
                &config,
                mode,
                SamplingStrategy::Center,
                &mut [],
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats
            ),
            Err(Error::ScratchTooSmall)
        );
        assert_eq!(tracker.state(), previous);
        assert_eq!(stats, Stats::default());
    }
}

#[test]
fn fast_dense_retry_recovers_a_small_rotated_symbol_with_shared_budgets() {
    let bits = include_bytes!("../../../fixtures/v1.bin");
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segs = [Segment::EMPTY; 8];
    // At this small scale the coarse rows miss a finder observation. Dense
    // scanning recovers it; use a deterministic inverse nearest-pixel rotation.
    let size = 43;
    let angle = 11.0f32 * core::f32::consts::PI / 180.0;
    let (sin, cos) = angle.sin_cos();
    let scale = size as f32 / 37.0;
    let pixels: Vec<_> = (0..size * size)
        .map(|i| {
            let fx = (i % size) as f32 + 0.5 - size as f32 * 0.5;
            let fy = (i / size) as f32 + 0.5 - size as f32 * 0.5 - 1.0 / 3.0;
            let sx = (fx * cos + fy * sin) / scale + 10.5;
            let sy = (-fx * sin + fy * cos) / scale + 10.5;
            if (0.0..21.0).contains(&sx) && (0.0..21.0).contains(&sy) {
                let n = sy as usize * 21 + sx as usize;
                if bits[n / 8] & (128 >> (n % 8)) != 0 {
                    return 0;
                }
            }
            255
        })
        .collect();
    let image = GrayImage::new(&pixels, size, size, size).unwrap();
    let mut reference = None;
    for detector_len in [
        config.scratch_bytes().unwrap(),
        config
            .scratch_bytes_for_mode(DetectionMode::Fast, size, size)
            .unwrap(),
    ] {
        ds.resize(detector_len, 0xff);
        let mut stats = Stats::default();
        let mut found = decode_with_mode(
            image,
            &config,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut output,
            &mut segs,
            &mut stats,
        )
        .unwrap();
        assert_eq!(&output[..found.decoded.payload_len], b"TEST");
        // Eight coarse passes, then the first dense global pass succeeds.
        assert_eq!(stats.passes, 9);
        found.stats.pixel_reads = 0;
        if let Some(previous) = reference {
            assert_eq!(found, previous);
        } else {
            reference = Some(found);
        }
        let limited = DefaultConfig {
            max_pixel_reads: stats.pixel_reads - 1,
            ..config
        };
        assert_eq!(
            decode_with_mode(
                image,
                &limited,
                DetectionMode::Fast,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segs,
                &mut stats
            ),
            Err(Error::PixelBudget)
        );
        assert_eq!(stats.pixel_reads, limited.max_pixel_reads);
        // Eight coarse passes, then the first dense global pass succeeds.
        assert_eq!(stats.passes, 9);
    }
}

#[test]
fn tracker_all_modes_recover_a_symbol_moved_outside_the_saved_roi() {
    let symbol = pgm(IMAGES[0]);
    let (width, height) = (400, 200);
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for mode in MODES {
        let mut tracker = Tracker::new();
        for offset in [5, 245] {
            let mut pixels = vec![255; width * height];
            for y in 0..symbol.height() {
                for x in 0..symbol.width() {
                    pixels[(y + 20) * width + x + offset] = symbol.pixel(x, y).unwrap();
                }
            }
            let image = GrayImage::new(&pixels, width, height, width).unwrap();
            let mut stats = Stats::default();
            let found = tracker
                .decode_with_mode(
                    image,
                    &config,
                    mode,
                    SamplingStrategy::Center,
                    &mut ds,
                    &mut cs,
                    &mut output,
                    &mut segments,
                    &mut stats,
                )
                .unwrap();
            assert_eq!(&output[..found.decoded.payload_len], b"TEST");
            assert!(found.corners.iter().all(|p| p.x > offset as f32));
            assert!(matches!(tracker.state(), TrackerState::Tracking { .. }));
            assert!(stats.pixel_reads <= config.max_pixel_reads);
            assert!(stats.passes <= if mode == DetectionMode::Fast { 16 } else { 8 });
        }
    }
}

#[test]
fn direct_preserves_search_counters_and_fixture_read_bounds() {
    // Search order and candidate work stay stable. Exact local decisions can
    // reduce reads; strong V1 range discovery visits 360 rows plus 31 header reads.
    let expected = [
        (50678, 145, 45, 1, 1),
        (68868, 165, 50, 1, 1),
        (182641, 265, 80, 1, 1),
        (411952, 405, 90, 1, 1),
        (622042, 459, 339, 1, 1),
        (921174, 555, 531, 1, 1),
        (295572, 360, 107, 1, 1),
        (298470, 360, 108, 3, 1),
        (310455, 360, 96, 1, 1),
        (294471, 360, 75, 1, 1),
        (314293, 360, 96, 1, 1),
        (4738506, 1080, 170, 1, 3),
    ];
    let config = DefaultConfig::default();
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for (file, (read_limit, rows, finder_observations, retries, passes)) in
        IMAGES.iter().zip(expected)
    {
        let mut stats = Stats::default();
        let found = decode_with_mode(
            pgm(file),
            &config,
            DetectionMode::Direct,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut output,
            &mut segments,
            &mut stats,
        )
        .unwrap();
        assert_eq!(&output[..found.decoded.payload_len], b"TEST");
        assert!(stats.pixel_reads <= read_limit);
        assert_eq!(
            stats,
            Stats {
                pixel_reads: stats.pixel_reads,
                rows,
                finder_observations,
                candidates: 1,
                retries,
                passes,
            }
        );
    }
}
