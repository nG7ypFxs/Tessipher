#![cfg(all(feature = "detect", tessipher_repository_fixtures))]

use tessipher::core::{Segment, SCRATCH_BYTES};
use tessipher::detect::*;

fn load(file: &[u8]) -> (usize, usize, Vec<u8>) {
    let mut lines = file.splitn(4, |&b| b == b'\n');
    assert_eq!(lines.next().unwrap(), b"P5");
    let mut sizes = core::str::from_utf8(lines.next().unwrap())
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse::<usize>().unwrap());
    let (w, h) = (sizes.next().unwrap(), sizes.next().unwrap());
    assert_eq!(lines.next().unwrap(), b"255");
    (w, h, lines.next().unwrap().to_vec())
}

// Compare observable decoding and search counters; actual input reads are
// deliberately different because cached classifications need no input reads.
fn normalized(result: Result<Detection, Error>) -> Result<Detection, Error> {
    result.map(|mut found| {
        found.stats.pixel_reads = 0;
        found
    })
}

#[test]
fn one_byte_short_memo_uses_minimal_path_and_preserves_unused_tail() {
    let (width, height, pixels) = load(include_bytes!("../../../fixtures/lighting-v2.pgm"));
    let config = DefaultConfig::default();
    let base = config.scratch_bytes().unwrap();
    let needed = config
        .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
        .unwrap();
    let mut minimal = vec![0; base];
    let mut short = vec![0xa5; needed + 7];
    let mut core = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let image = GrayImage::new(&pixels, width, height, width).unwrap();
    for max_pixel_reads in [500_000, usize::MAX] {
        let limited = DefaultConfig {
            max_pixel_reads,
            ..config
        };
        let mut reference_stats = Stats::default();
        let reference = decode_with_mode(
            image,
            &limited,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut minimal,
            &mut core,
            &mut output,
            &mut segments,
            &mut reference_stats,
        );
        let expected_payload = reference
            .as_ref()
            .ok()
            .map(|f| output[..f.decoded.payload_len].to_vec());
        let mut stats = Stats::default();
        let actual = decode_with_mode(
            image,
            &limited,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut short[..needed - 1],
            &mut core,
            &mut output,
            &mut segments,
            &mut stats,
        );
        assert_eq!(actual, reference);
        assert_eq!(stats, reference_stats);
        if let Some(expected) = expected_payload {
            assert_eq!(&output[..expected.len()], expected);
        }
        assert!(short[base..].iter().all(|&byte| byte == 0xa5));
    }
}

#[test]
fn memo_resets_between_different_threshold_roi_and_full_frame_recovery() {
    let (symbol_width, symbol_height, symbol) =
        load(include_bytes!("../../../fixtures/lighting-v2.pgm"));
    let (width, height) = (512, 400);
    let mut pixels = vec![255; width * height];
    for y in 0..symbol_height {
        pixels[(y + 20) * width + 128..(y + 20) * width + 128 + symbol_width]
            .copy_from_slice(&symbol[y * symbol_width..(y + 1) * symbol_width]);
    }
    // This finderless ROI reaches local passes with a different min/max threshold.
    for y in 0..64 {
        for x in 0..64 {
            pixels[y * width + x] = (32 + x) as u8;
        }
    }
    let image = GrayImage::new(&pixels, width, height, width).unwrap();
    let wrong_roi = TrackHint {
        roi: Rect {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        },
        image_width: width,
        image_height: height,
    };
    let config = DefaultConfig::default();
    let mut core = [0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let mut reference = None;
    let mut reference_state = None;
    for size in [
        config.scratch_bytes().unwrap(),
        config
            .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
            .unwrap(),
    ] {
        let mut scratch = vec![0xff; size];
        let mut tracker = Tracker::from_state(TrackerState::Tracking { hint: wrong_roi }).unwrap();
        let mut stats = Stats::default();
        let result = tracker
            .decode_with_mode(
                image,
                &config,
                DetectionMode::Fast,
                SamplingStrategy::Center,
                &mut scratch,
                &mut core,
                &mut output,
                &mut segments,
                &mut stats,
            )
            .unwrap();
        assert_eq!(&output[..result.decoded.payload_len], b"TEST");
        assert!(result.corners.iter().all(|point| point.x > 128.0));
        // Eight failed ROI passes precede a successful full-frame pass.
        assert!(stats.passes > 8);
        if let Some(expected) = reference {
            assert_eq!(normalized(Ok(result)), expected);
            assert_eq!(Some(tracker.state()), reference_state);
        } else {
            reference = Some(normalized(Ok(result)));
            reference_state = Some(tracker.state());
        }
    }
}

#[test]
fn memo_preserves_decoding_for_reused_storage_inverted_and_strided_images() {
    let files: &[&[u8]] = &[
        include_bytes!("../../../fixtures/v1.pgm"),
        include_bytes!("../../../fixtures/v7.pgm"),
        include_bytes!("../../../fixtures/v40.pgm"),
        include_bytes!("../../../fixtures/lighting-v2.pgm"),
        include_bytes!("../../../fixtures/perspective-strong-v1.pgm"),
        include_bytes!("../../../fixtures/rotated45-v2.pgm"),
    ];
    let config = DefaultConfig::default();
    let base = config.scratch_bytes().unwrap();
    let mut minimal = vec![0; base];
    let mut expanded = vec![
        0xff;
        config
            .scratch_bytes_for_mode(DetectionMode::Fast, 640, 640)
            .unwrap()
            + 9
    ];
    let mut core = [0; SCRATCH_BYTES];
    let mut out = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for file in files {
        let (width, height, source) = load(file);
        let stride = width + 11;
        let needed = config
            .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
            .unwrap();
        for inverted in [false, true] {
            let mut pixels = vec![71; (height - 1) * stride + width];
            for y in 0..height {
                for x in 0..width {
                    pixels[y * stride + x] = source[y * width + x] ^ if inverted { 255 } else { 0 };
                }
            }
            let image = GrayImage::new(&pixels, width, height, stride).unwrap();
            for strategy in [
                SamplingStrategy::Center,
                SamplingStrategy::CenterThenMajority5,
            ] {
                let mut reference_stats = Stats::default();
                let reference = decode_with_mode(
                    image,
                    &config,
                    DetectionMode::Fast,
                    strategy,
                    &mut minimal,
                    &mut core,
                    &mut out,
                    &mut segments,
                    &mut reference_stats,
                );
                let expected_payload = reference
                    .as_ref()
                    .ok()
                    .map(|f| out[..f.decoded.payload_len].to_vec());
                expanded[needed..needed + 9].fill(0x5a);
                let mut stats = Stats::default();
                let actual = decode_with_mode(
                    image,
                    &config,
                    DetectionMode::Fast,
                    strategy,
                    &mut expanded[..needed + 9],
                    &mut core,
                    &mut out,
                    &mut segments,
                    &mut stats,
                );
                if let Some(expected) = expected_payload {
                    assert_eq!(
                        &out[..actual.as_ref().unwrap().decoded.payload_len],
                        expected
                    );
                }
                assert_eq!(normalized(actual), normalized(reference));
                assert!(stats.pixel_reads <= reference_stats.pixel_reads);
                stats.pixel_reads = 0;
                reference_stats.pixel_reads = 0;
                assert_eq!(stats, reference_stats);
                assert!(expanded[needed..needed + 9].iter().all(|&v| v == 0x5a));
            }
        }
    }
}

#[test]
fn fast_memo_tracks_roi_transitions_and_respects_finite_budgets() {
    let (width, height, pixels) = load(include_bytes!("../../../fixtures/lighting-v2.pgm"));
    let config = DefaultConfig::default();
    let mut minimal = vec![0; config.scratch_bytes().unwrap()];
    let mut expanded = vec![
        0xff;
        config
            .scratch_bytes_for_mode(DetectionMode::Fast, width, height)
            .unwrap()
    ];
    let mut core = [0; SCRATCH_BYTES];
    let mut out = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    let blank = vec![255; pixels.len()];
    let inverted: Vec<_> = pixels.iter().map(|&v| 255 - v).collect();
    let mut baseline = Tracker::new();
    let mut cached = Tracker::new();
    for input in [&pixels, &pixels, &inverted, &blank, &pixels, &blank, &blank] {
        let image = GrayImage::new(input, width, height, width).unwrap();
        let mut reference_stats = Stats::default();
        let reference = baseline.decode_with_mode(
            image,
            &config,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut minimal,
            &mut core,
            &mut out,
            &mut segments,
            &mut reference_stats,
        );
        let mut stats = Stats::default();
        let actual = cached.decode_with_mode(
            image,
            &config,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut expanded,
            &mut core,
            &mut out,
            &mut segments,
            &mut stats,
        );
        assert_eq!(normalized(actual), normalized(reference));
        assert_eq!(cached.state(), baseline.state());
    }
    let image = GrayImage::new(&pixels, width, height, width).unwrap();
    for limit in [1, width * height, 300_000, 500_000, 1_000_000] {
        let limited = DefaultConfig {
            max_pixel_reads: limit,
            ..config
        };
        let mut stats = Stats::default();
        let result = decode_with_mode(
            image,
            &limited,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut expanded,
            &mut core,
            &mut out,
            &mut segments,
            &mut stats,
        );
        assert!(stats.pixel_reads <= limit);
        if result == Err(Error::PixelBudget) {
            assert_eq!(stats.pixel_reads, limit);
        }
    }
}
