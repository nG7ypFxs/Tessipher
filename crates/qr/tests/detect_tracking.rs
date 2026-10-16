#![cfg(feature = "detect")]

use tessipher::core::{Segment, SCRATCH_BYTES};
use tessipher::detect::*;

fn symbol(scale: usize, damaged_centers: bool, bad_data: bool) -> (Vec<u8>, usize) {
    let bits = include_bytes!("../../../fixtures/v1.bin");
    let width = 21 * scale;
    let mut pixels = vec![255; width * width];
    let mut rng = 0x12345678u32;
    for y in 0..21 {
        for x in 0..21 {
            let i = y * 21 + x;
            let mut dark = bits[i / 8] & (128 >> (i % 8)) != 0;
            if bad_data && !tessipher::core::is_function(1, x, y) {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                dark = rng & 0x8000_0000 != 0;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    pixels[(y * scale + dy) * width + x * scale + dx] = if dark { 0 } else { 255 };
                }
            }
            if damaged_centers && !tessipher::core::is_function(1, x, y) {
                for dy in scale / 2 - 1..=scale / 2 {
                    for dx in scale / 2 - 1..=scale / 2 {
                        pixels[(y * scale + dy) * width + x * scale + dx] =
                            if dark { 255 } else { 0 };
                    }
                }
            }
        }
    }
    (pixels, width)
}
fn frame(symbol: &[u8], side: usize, width: usize, height: usize, x: usize, y: usize) -> Vec<u8> {
    let mut pixels = vec![255; width * height];
    for row in 0..side {
        pixels[(row + y) * width + x..(row + y) * width + x + side]
            .copy_from_slice(&symbol[row * side..(row + 1) * side]);
    }
    pixels
}
fn call(
    image: GrayImage<'_>,
    config: &DefaultConfig,
    strategy: SamplingStrategy,
    tracker: Option<&mut Tracker>,
    capacity: bool,
) -> (Result<Detection, Error>, Stats) {
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = vec![0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let mut stats = Stats::default();
    let out = if capacity {
        &mut output[..]
    } else {
        &mut output[..0]
    };
    let result = match tracker {
        Some(t) => t.decode_with_stats(
            image,
            config,
            strategy,
            &mut ds,
            &mut cs,
            out,
            &mut segments,
            &mut stats,
        ),
        None => decode_with_sampling(
            image,
            config,
            strategy,
            &mut ds,
            &mut cs,
            out,
            &mut segments,
            &mut stats,
        ),
    };
    if let Ok(found) = result {
        assert_eq!(found.stats, stats);
        assert_eq!(&output[..found.decoded.payload_len], b"TEST");
    }
    (result, stats)
}
fn view(p: &[u8], w: usize, h: usize) -> GrayImage<'_> {
    GrayImage::new(p, w, h, w).unwrap()
}

#[test]
fn majority_recovers_center_damage_and_success_is_identical_to_legacy() {
    let config = DefaultConfig::default();
    let (plain, side) = symbol(8, false, false);
    let (damaged, _) = symbol(8, true, false);
    for mirrored in [false, true] {
        for inverted in [false, true] {
            let transform = |symbol: &[u8]| {
                let mut p = frame(symbol, side, 240, 240, 32, 32);
                if mirrored {
                    for row in p.chunks_exact_mut(240) {
                        row.reverse();
                    }
                }
                if inverted {
                    for v in &mut p {
                        *v = 255 - *v;
                    }
                }
                p
            };
            let p = transform(&plain);
            let center = call(
                view(&p, 240, 240),
                &config,
                SamplingStrategy::Center,
                None,
                true,
            )
            .0
            .unwrap();
            let majority = call(
                view(&p, 240, 240),
                &config,
                SamplingStrategy::CenterThenMajority5,
                None,
                true,
            )
            .0
            .unwrap();
            assert_eq!(center, majority);
            let p = transform(&damaged);
            assert!(call(
                view(&p, 240, 240),
                &DefaultConfig {
                    max_retries: 1,
                    ..config
                },
                SamplingStrategy::Center,
                None,
                true
            )
            .0
            .is_err());
            let (found, stats) = call(
                view(&p, 240, 240),
                &config,
                SamplingStrategy::CenterThenMajority5,
                None,
                true,
            );
            let found = found.unwrap();
            if !mirrored && !inverted {
                let (center, center_stats) = call(
                    view(&p, 240, 240),
                    &config,
                    SamplingStrategy::Center,
                    None,
                    true,
                );
                eprintln!("damaged default center={:?} center_reads={} center_retries={} majority_reads={} majority_retries={}", center.map(|d| d.decoded.version), center_stats.pixel_reads, center_stats.retries, stats.pixel_reads, stats.retries);
            }
            assert_eq!(stats.retries, 2);
            assert_eq!(found.inverted, inverted);
            let exact = DefaultConfig {
                max_pixel_reads: stats.pixel_reads,
                ..config
            };
            assert!(call(
                view(&p, 240, 240),
                &exact,
                SamplingStrategy::CenterThenMajority5,
                None,
                true
            )
            .0
            .is_ok());
            let short = DefaultConfig {
                max_pixel_reads: stats.pixel_reads - 1,
                ..config
            };
            let (r, s) = call(
                view(&p, 240, 240),
                &short,
                SamplingStrategy::CenterThenMajority5,
                None,
                true,
            );
            assert_eq!(r, Err(Error::PixelBudget));
            assert_eq!(s.pixel_reads, short.max_pixel_reads);
            let once = DefaultConfig {
                max_retries: 1,
                ..config
            };
            assert_eq!(
                call(
                    view(&p, 240, 240),
                    &once,
                    SamplingStrategy::CenterThenMajority5,
                    None,
                    true
                )
                .0,
                Err(Error::RetryBudget)
            );
        }
    }
}

#[test]
fn tracker_sequence_coordinates_work_and_frame_budget() {
    let config = DefaultConfig::default();
    let (symbol, side) = symbol(4, false, false);
    let p = frame(&symbol, side, 400, 300, 40, 60);
    let input = view(&p, 400, 300);
    let mut tracker = Tracker::new();
    let full = call(input, &config, SamplingStrategy::Center, None, true)
        .0
        .unwrap();
    let first = call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    assert_eq!(full, first);
    assert!(matches!(tracker.state(), TrackerState::Tracking { .. }));
    let second = call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    for (a, b) in full.corners.iter().zip(second.corners) {
        assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01);
    }
    assert!(second.stats.pixel_reads < full.stats.pixel_reads / 3);
    eprintln!(
        "tracker bytes={} first_reads={} stable_reads={}",
        std::mem::size_of::<Tracker>(),
        full.stats.pixel_reads,
        second.stats.pixel_reads
    );
    let p = frame(&symbol, side, 400, 300, 44, 64);
    assert!(call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true
    )
    .0
    .is_ok());
    let saved = tracker;
    let p = frame(&symbol, side, 400, 300, 280, 180);
    let (recovered, recovery_stats) = call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    );
    assert!(recovered.is_ok());
    assert!(recovery_stats.pixel_reads > full.stats.pixel_reads);
    let mut exact_tracker = saved;
    let exact = DefaultConfig {
        max_pixel_reads: recovery_stats.pixel_reads,
        ..config
    };
    assert!(call(
        view(&p, 400, 300),
        &exact,
        SamplingStrategy::Center,
        Some(&mut exact_tracker),
        true
    )
    .0
    .is_ok());
    let mut short_tracker = saved;
    let short = DefaultConfig {
        max_pixel_reads: recovery_stats.pixel_reads - 1,
        ..config
    };
    let (r, s) = call(
        view(&p, 400, 300),
        &short,
        SamplingStrategy::Center,
        Some(&mut short_tracker),
        true,
    );
    assert_eq!(r, Err(Error::PixelBudget));
    assert_eq!(s.pixel_reads, short.max_pixel_reads);
    assert_eq!(short_tracker, saved);
    let blank = vec![255; 400 * 300];
    assert_eq!(
        call(
            view(&blank, 400, 300),
            &config,
            SamplingStrategy::Center,
            Some(&mut tracker),
            true
        )
        .0,
        Err(Error::NotFound)
    );
    assert!(matches!(
        tracker.state(),
        TrackerState::Candidate { misses: 1, .. }
    ));
    assert_eq!(
        call(
            view(&blank, 400, 300),
            &config,
            SamplingStrategy::Center,
            Some(&mut tracker),
            true
        )
        .0,
        Err(Error::NotFound)
    );
    assert_eq!(tracker.state(), TrackerState::Searching);
    assert!(call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true
    )
    .0
    .is_ok());
    tracker.reset();
    assert_eq!(tracker.state(), TrackerState::Searching);
}

#[test]
fn unconfirmed_candidates_expire_and_terminal_errors_are_transactional() {
    let config = DefaultConfig::default();
    let (bad, side) = symbol(4, false, true);
    let p = frame(&bad, side, 400, 300, 40, 60);
    let mut tracker = Tracker::new();
    assert!(call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true
    )
    .0
    .is_err());
    assert!(matches!(
        tracker.state(),
        TrackerState::Candidate { misses: 1, .. }
    ));
    let candidate = tracker;
    assert!(call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true
    )
    .0
    .is_err());
    assert_eq!(tracker.state(), TrackerState::Searching);
    // Failed ROI plus full-image work uses all eight passes and never resets counters.
    let mut twice = candidate;
    let (_, combined) = call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut twice),
        true,
    );
    assert_eq!(combined.passes, 8);
    tracker = candidate;
    let (good, _) = symbol(4, false, false);
    let p = frame(&good, side, 400, 300, 40, 60);
    assert!(call(
        view(&p, 400, 300),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true
    )
    .0
    .is_ok());
    let saved = tracker;
    assert_eq!(
        call(
            view(&p, 400, 300),
            &config,
            SamplingStrategy::CenterThenMajority5,
            Some(&mut tracker),
            false
        )
        .0,
        Err(Error::Decode(tessipher::core::Error::OutputTooSmall))
    );
    assert_eq!(tracker, saved);
    let bad_cfg = DefaultConfig {
        max_pixel_reads: 1,
        ..config
    };
    assert_eq!(
        call(
            view(&p, 400, 300),
            &bad_cfg,
            SamplingStrategy::Center,
            Some(&mut tracker),
            true
        )
        .0,
        Err(Error::PixelBudget)
    );
    assert_eq!(tracker, saved);
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = vec![0; SCRATCH_BYTES];
    let mut stats = Stats::default();
    assert_eq!(
        tracker.decode_with_stats(
            view(&p, 400, 300),
            &Config::<399, 300>::default(),
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut [],
            &mut [],
            &mut stats
        ),
        Err(Error::ImageTooLarge)
    );
    assert_eq!(stats, Stats::default());
    assert_eq!(tracker, saved);
    assert_eq!(
        tracker.decode_with_stats(
            view(&p, 400, 300),
            &config,
            SamplingStrategy::Center,
            &mut [],
            &mut cs,
            &mut [],
            &mut [],
            &mut stats
        ),
        Err(Error::ScratchTooSmall)
    );
    assert_eq!(tracker, saved);
    let smaller = frame(&good, side, 300, 240, 160, 120);
    let full = call(
        view(&smaller, 300, 240),
        &config,
        SamplingStrategy::Center,
        None,
        true,
    )
    .0
    .unwrap();
    let tracked = call(
        view(&smaller, 300, 240),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    assert_eq!(tracked, full);
}

#[test]
fn subregions_preserve_stride_check_overflow_and_need_no_last_row_padding() {
    let pixels: Vec<u8> = (0..34).collect();
    let parent = GrayImage::new(&pixels, 4, 4, 10).unwrap();
    for rect in [
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        },
        Rect {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        },
        Rect {
            x: 0,
            y: 2,
            width: 2,
            height: 2,
        },
        Rect {
            x: 2,
            y: 2,
            width: 2,
            height: 2,
        },
    ] {
        let roi = parent.sub_roi(rect).unwrap();
        assert_eq!(roi.stride(), 10);
        assert_eq!(roi.pixel(0, 0), parent.pixel(rect.x, rect.y));
        assert_eq!(roi.pixel(1, 1), parent.pixel(rect.x + 1, rect.y + 1));
    }
    for rect in [
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Rect {
            x: 0,
            y: 4,
            width: 1,
            height: 1,
        },
        Rect {
            x: usize::MAX,
            y: 0,
            width: 1,
            height: 1,
        },
        Rect {
            x: 0,
            y: usize::MAX,
            width: 1,
            height: 1,
        },
    ] {
        assert!(matches!(parent.sub_roi(rect), Err(Error::InvalidImage)));
    }
}

#[test]
fn opt_in_supports_perspective_and_local_threshold_and_mirror_roi_origin() {
    let config = DefaultConfig::default();
    for file in [
        include_bytes!("../../../fixtures/perspective-strong-v1.pgm").as_slice(),
        include_bytes!("../../../fixtures/lighting-v2.pgm").as_slice(),
    ] {
        let offset = file
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(2)
            .unwrap()
            .0
            + 1;
        let header = std::str::from_utf8(&file[..offset]).unwrap();
        let mut words = header.split_whitespace();
        words.next();
        let w = words.next().unwrap().parse().unwrap();
        let h = words.next().unwrap().parse().unwrap();
        assert!(call(
            view(&file[offset..], w, h),
            &config,
            SamplingStrategy::CenterThenMajority5,
            None,
            true
        )
        .0
        .is_ok());
    }
    let (symbol, side) = symbol(4, false, false);
    let mut p = frame(&symbol, side, 400, 300, 40, 60);
    for row in p.chunks_exact_mut(400) {
        row.reverse();
    }
    let input = view(&p, 400, 300);
    let mut tracker = Tracker::new();
    let full = call(input, &config, SamplingStrategy::Center, None, true)
        .0
        .unwrap();
    call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    let saved = tracker;
    let roi = call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    for (a, b) in full.corners.iter().zip(roi.corners) {
        assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01);
    }
    // Identical coordinates and dimensions with a different row stride retain the hint.
    let mut padded = vec![255; 299 * 420 + 400];
    for y in 0..300 {
        padded[y * 420..y * 420 + 400].copy_from_slice(&p[y * 400..(y + 1) * 400]);
    }
    tracker = saved;
    let r = call(
        GrayImage::new(&padded, 400, 300, 420).unwrap(),
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    assert_eq!(r, roi);
}

#[test]
fn a_saved_full_image_roi_is_not_searched_twice() {
    let config = DefaultConfig::default();
    let (symbol, side) = symbol(4, false, false);
    let pixels = frame(&symbol, side, 120, 120, 18, 18);
    let input = view(&pixels, 120, 120);
    let mut tracker = Tracker::new();
    let first = call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    let TrackerState::Tracking { hint } = tracker.state() else {
        panic!("missing tracking hint")
    };
    assert_eq!(
        hint.roi,
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 120
        }
    );
    let second = call(
        input,
        &config,
        SamplingStrategy::Center,
        Some(&mut tracker),
        true,
    )
    .0
    .unwrap();
    assert_eq!(first, second);
}

#[test]
fn safe_tracker_restore_roundtrips_valid_states_and_rejects_invalid_hints() {
    let hint = TrackHint {
        roi: Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 80,
        },
        image_width: 400,
        image_height: 300,
    };
    for state in [
        TrackerState::Searching,
        TrackerState::Candidate { hint, misses: 1 },
        TrackerState::Tracking { hint },
    ] {
        assert_eq!(Tracker::from_state(state).unwrap().state(), state);
    }
    for misses in [0, 2, u8::MAX] {
        assert_eq!(
            Tracker::from_state(TrackerState::Candidate { hint, misses }),
            Err(Error::InvalidConfig)
        );
    }
    let mut invalid = [hint; 8];
    invalid[0].roi.width = 0;
    invalid[1].roi.height = 0;
    invalid[2].roi.x = usize::MAX;
    invalid[3].roi.y = usize::MAX;
    invalid[4].image_width = 0;
    invalid[5].image_height = 32769;
    invalid[6].roi.x = 400;
    invalid[7].roi.y = 300;
    for hint in invalid {
        assert_eq!(
            Tracker::from_state(TrackerState::Tracking { hint }),
            Err(Error::InvalidImage)
        );
    }
}

#[test]
fn cached_tracker_reuses_storage_and_shares_roi_full_budget() {
    let config = DefaultConfig {
        max_pixel_reads: usize::MAX,
        ..Default::default()
    };
    let (qr, side) = symbol(4, false, false);
    let stable = frame(&qr, side, 320, 240, 24, 24);
    let moved = frame(&qr, side, 320, 240, 200, 140);
    let blank = vec![255; 320 * 240];
    let mut raw_tracker = Tracker::new();
    let mut cached_tracker = Tracker::new();
    let mut cache = vec![0xa5; 5 * 640 + 1];
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let mut stats = Stats::default();
    // A moved symbol forces ROI failure followed by full-frame success; two
    // blank frames exercise failed-state commits, then stride/dimension changes.
    for (pixels, w, h, stride) in [
        (&stable[..], 320, 240, 320),
        (&stable[..], 320, 240, 320),
        (&moved[..], 320, 240, 320),
        (&blank[..], 320, 240, 320),
        (&blank[..], 320, 240, 320),
        (&qr[..], side, side, side),
        (&stable[..], 310, 230, 320),
    ] {
        let image = GrayImage::new(pixels, w, h, stride).unwrap();
        let (raw, raw_stats) = call(
            image,
            &config,
            SamplingStrategy::Center,
            Some(&mut raw_tracker),
            true,
        );
        let cached = cached_tracker.decode_with_stats_cached(
            image,
            &config,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut output,
            &mut segments,
            &mut stats,
            &mut cache,
        );
        assert_eq!(raw_tracker.state(), cached_tracker.state());
        // These frames succeed globally. Direct can finish exact 0/255 range
        // discovery early; the explicit row-cache API retains scalar discovery.
        assert!(raw_stats.pixel_reads <= stats.pixel_reads);
        let mut normalized_stats = stats;
        normalized_stats.pixel_reads = raw_stats.pixel_reads;
        assert_eq!(normalized_stats, raw_stats);
        let normalized = cached.map(|mut found| {
            assert_eq!(found.stats, stats);
            found.stats.pixel_reads = raw_stats.pixel_reads;
            found
        });
        assert_eq!(raw, normalized);
        if let Ok(found) = cached {
            assert_eq!(&output[..found.decoded.payload_len], b"TEST");
        }
    }
    assert_eq!(cache[5 * 640], 0xa5);
    // Terminal budget/capacity rejection must preserve state, even after ROI fails.
    let image = view(&moved, 320, 240);
    let hint = TrackHint {
        roi: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        },
        image_width: 320,
        image_height: 240,
    };
    let initial = TrackerState::Tracking { hint };
    for limit in [1, 10000, 10001, 10000 + 320 * 240 - 1] {
        let cfg = DefaultConfig {
            max_pixel_reads: limit,
            ..config
        };
        let mut tracker = Tracker::from_state(initial).unwrap();
        assert_eq!(
            tracker.decode_with_stats_cached(
                image,
                &cfg,
                SamplingStrategy::Center,
                &mut ds,
                &mut cs,
                &mut output,
                &mut segments,
                &mut stats,
                &mut cache
            ),
            Err(Error::PixelBudget)
        );
        assert_eq!(stats.pixel_reads, limit);
        assert_eq!(tracker.state(), initial);
    }
    let mut tracker = Tracker::from_state(initial).unwrap();
    assert_eq!(
        tracker.decode_with_stats_cached(
            image,
            &config,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut [],
            &mut segments,
            &mut stats,
            &mut cache
        ),
        Err(Error::Decode(tessipher::core::Error::OutputTooSmall))
    );
    assert_eq!(tracker.state(), initial);
}

#[test]
fn cached_tracker_local_roi_full_attempts_have_fresh_tags() {
    let file = include_bytes!("../../../fixtures/lighting-v2.pgm");
    let offset = file
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(2)
        .unwrap()
        .0
        + 1;
    let pixels = &file[offset..];
    let image = view(pixels, 360, 360);
    let cfg = DefaultConfig {
        max_pixel_reads: DEFAULT_MAX_PIXEL_READS,
        ..Default::default()
    };
    let hint = TrackHint {
        roi: Rect {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        },
        image_width: 360,
        image_height: 360,
    };
    let initial = TrackerState::Tracking { hint };
    let mut raw_tracker = Tracker::from_state(initial).unwrap();
    let mut tracker = Tracker::from_state(initial).unwrap();
    let (raw, raw_stats) = call(
        image,
        &cfg,
        SamplingStrategy::Center,
        Some(&mut raw_tracker),
        true,
    );
    let mut ds = vec![0; cfg.scratch_bytes().unwrap()];
    let mut cs = [0; SCRATCH_BYTES];
    let mut out = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let mut cache = vec![0xa5; 5 * 360];
    let mut stats = Stats::default();
    let cached = tracker
        .decode_with_stats_cached(
            image,
            &cfg,
            SamplingStrategy::Center,
            &mut ds,
            &mut cs,
            &mut out,
            &mut segments,
            &mut stats,
            &mut cache,
        )
        .unwrap();
    let raw = raw.unwrap();
    assert!(raw.local_threshold);
    assert!(raw_stats.passes > 3);
    assert!(cache.iter().any(|&b| b != 0xa5));
    assert!(stats.pixel_reads <= cfg.max_pixel_reads);
    assert!(raw_stats.pixel_reads <= cfg.max_pixel_reads);
    let mut normalized = cached;
    normalized.stats.pixel_reads = raw.stats.pixel_reads;
    assert_eq!(raw, normalized);
    assert_eq!(tracker.state(), raw_tracker.state());
    assert_eq!(&out[..cached.decoded.payload_len], b"TEST");
    // On the same buffer, another ROI/full frame with no symbol must commit
    // recognition failure, never decode data left over from the previous frame.
    let blank: Vec<_> = (0..360 * 360)
        .map(|i| ((i * 17 + i / 360 * 31) % 256) as u8)
        .collect();
    let image = view(&blank, 360, 360);
    let (raw, raw_stats) = call(
        image,
        &cfg,
        SamplingStrategy::Center,
        Some(&mut raw_tracker),
        true,
    );
    let cached = tracker.decode_with_stats_cached(
        image,
        &cfg,
        SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut segments,
        &mut stats,
        &mut cache,
    );
    assert_eq!(raw, cached);
    assert!(raw.is_err());
    assert_eq!(tracker.state(), raw_tracker.state());
    assert!(stats.pixel_reads <= cfg.max_pixel_reads);
    assert!(raw_stats.pixel_reads <= cfg.max_pixel_reads);
    stats.pixel_reads = raw_stats.pixel_reads;
    assert_eq!(stats, raw_stats);
}
