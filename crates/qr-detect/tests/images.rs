use qr_core::{Segment, SCRATCH_BYTES};
use qr_detect::*;
fn image_rect(v: u8, scale_x: usize, scale_y: usize) -> (Vec<u8>, usize, usize) {
    let bits: &[u8] = match v {
        1 => include_bytes!("../../../fixtures/v1.bin"),
        2 => include_bytes!("../../../fixtures/v2.bin"),
        7 => include_bytes!("../../../fixtures/v7.bin"),
        14 => include_bytes!("../../../fixtures/v14.bin"),
        32 => include_bytes!("../../../fixtures/v32.bin"),
        _ => include_bytes!("../../../fixtures/v40.bin"),
    };
    let n = v as usize * 4 + 17;
    let w = (n + 8) * scale_x;
    let h = (n + 8) * scale_y;
    let mut pixels = vec![255; w * h];
    for y in 0..n {
        for x in 0..n {
            let p = y * n + x;
            if bits[p / 8] & (128 >> (p % 8)) != 0 {
                for j in 0..scale_y {
                    for i in 0..scale_x {
                        pixels[((y + 4) * scale_y + j) * w + (x + 4) * scale_x + i] = 0;
                    }
                }
            }
        }
    }
    (pixels, w, h)
}
fn image(v: u8, scale: usize) -> (Vec<u8>, usize) {
    let (pixels, w, _) = image_rect(v, scale, scale);
    (pixels, w)
}
fn run<const W: usize, const H: usize>(
    pixels: &[u8],
    w: usize,
    h: usize,
    stride: usize,
    config: &Config<W, H>,
) -> Result<Detection, Error> {
    let image = GrayImage::new(pixels, w, h, stride)?;
    let mut ds = vec![0; config.scratch_bytes()?];
    let mut cs = vec![0; SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segs = [Segment::EMPTY; 8];
    let r = decode(image, config, &mut ds, &mut cs, &mut output, &mut segs)?;
    assert_eq!(&output[..r.decoded.payload_len], b"TEST");
    Ok(r)
}
#[test]
fn versions_mirrors_rotations_inversion_and_borders() {
    let config = DefaultConfig::default();
    for v in [1, 2, 7, 14, 32, 40] {
        let (original, w) = image(v, if v >= 32 { 3 } else { 4 });
        for mirrored in [false, true] {
            let mut pixels = original.clone();
            if mirrored {
                mirror_rows(&mut pixels, w);
            }
            for rotation in 0..4 {
                for inverted in [false, true] {
                    let input: Vec<_> = pixels
                        .iter()
                        .map(|&p| if inverted { 255 - p } else { p })
                        .collect();
                    let found = run(&input, w, w, w, &config).unwrap_or_else(|e| {
                        panic!(
                            "v{v},mirrored{mirrored},rotation{rotation},inverted{inverted}: {e:?}"
                        )
                    });
                    assert_eq!(found.decoded.version, v);
                    assert_eq!(found.inverted, inverted);
                }
                let mut rotated = vec![0; pixels.len()];
                for y in 0..w {
                    for x in 0..w {
                        rotated[x * w + w - y - 1] = pixels[y * w + x];
                    }
                }
                pixels = rotated;
            }
        }
    }
}
fn mirror_rows(pixels: &mut [u8], width: usize) {
    for row in pixels.chunks_exact_mut(width) {
        row.reverse();
    }
}
#[test]
fn mirror_uses_one_sampling_retry() {
    let (mut pixels, width) = image(1, 4);
    let config = DefaultConfig {
        max_retries: 1,
        ..Default::default()
    };
    let normal = run(&pixels, width, width, width, &config).unwrap();
    mirror_rows(&mut pixels, width);
    let mirrored = run(&pixels, width, width, width, &config).unwrap();
    assert_eq!(normal.stats.retries, 1);
    assert_eq!(mirrored.stats.retries, 1);
    for (i, original_index) in [0, 3, 2, 1].into_iter().enumerate() {
        let expected = normal.corners[original_index];
        assert!((mirrored.corners[i].x - (width as f32 - expected.x)).abs() < 0.001);
        assert!((mirrored.corners[i].y - expected.y).abs() < 0.001);
    }
}
#[test]
fn rectangular_roi_stride_1920() {
    let (pixels, w) = image(2, 5);
    let height = 240;
    let mut roi = vec![127; (height - 1) * 1920 + 640];
    for y in 0..height {
        for x in 0..640 {
            roi[y * 1920 + x] = 255;
        }
    }
    for y in 0..w {
        roi[(y + 30) * 1920 + 200..(y + 30) * 1920 + 200 + w]
            .copy_from_slice(&pixels[y * w..(y + 1) * w]);
    }
    run(&roi, 640, height, 1920, &Config::<640, 240>::default()).unwrap();
    assert_eq!(
        run(&roi, 640, height, 1920, &Config::<639, 240>::default()),
        Err(Error::ImageTooLarge)
    );
}
#[test]
fn invalid_and_memory_contracts() {
    assert!(GrayImage::new(&[], usize::MAX, 2, usize::MAX).is_err());
    assert!(GrayImage::new(&[0; 12], 4, 3, 5).is_err());
    assert!(GrayImage::new(&[0; 14], 4, 3, 5).is_ok());
    assert!(GrayImage::new(&[0; 12], 4, 3, 3).is_err());
    assert_eq!(
        Config::<320, 640>::default().scratch_bytes(),
        Config::<640, 320>::default().scratch_bytes()
    );
    assert_eq!(
        run(&[255; 64], 8, 8, 8, &DefaultConfig::default()),
        Err(Error::NotFound)
    );
    let cfg = DefaultConfig {
        max_retries: 0,
        ..Default::default()
    };
    assert_eq!(cfg.scratch_bytes(), Err(Error::InvalidConfig));
}
#[test]
fn arbitrary_rotation_perspective_and_lighting() {
    for (name, file) in [
        (
            "rotation45",
            include_bytes!("../../../fixtures/rotated45-v2.pgm").as_slice(),
        ),
        (
            "perspective-strong-v1",
            include_bytes!("../../../fixtures/perspective-strong-v1.pgm").as_slice(),
        ),
        (
            "rotation17",
            include_bytes!("../../../fixtures/rotated-v2.pgm").as_slice(),
        ),
        (
            "perspective-v1",
            include_bytes!("../../../fixtures/perspective-v1.pgm").as_slice(),
        ),
        (
            "perspective-v7",
            include_bytes!("../../../fixtures/perspective-v7.pgm").as_slice(),
        ),
        (
            "lighting",
            include_bytes!("../../../fixtures/lighting-v2.pgm").as_slice(),
        ),
    ] {
        let offset = file
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .nth(2)
            .unwrap()
            .0
            + 1;
        let mut mirrored = file[offset..].to_vec();
        mirror_rows(&mut mirrored, 360);
        run(&mirrored, 360, 360, 360, &DefaultConfig::default())
            .unwrap_or_else(|e| panic!("mirrored {name}: {e:?}"));
        let r = run(&file[offset..], 360, 360, 360, &DefaultConfig::default())
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        eprintln!("{name}: {:?}, local {}", r.stats, r.local_threshold);
        if name == "lighting" {
            assert!(r.local_threshold);
            assert_eq!(
                r.stats,
                Stats {
                    rows: 1080,
                    finder_observations: 170,
                    candidates: 1,
                    retries: 1,
                    passes: 3,
                }
            );
        }
        if name == "perspective-strong-v1" {
            assert_eq!(
                r.stats,
                Stats {
                    rows: 360,
                    finder_observations: 108,
                    candidates: 1,
                    retries: 3,
                    passes: 1,
                }
            );
        }
    }
}
#[test]
fn deterministic_budget_exhaustion() {
    let mut pixels = vec![255; 240 * 60];
    for k in 0..5 {
        for y in 0..7 {
            for x in 0..7 {
                let edge = x.min(6 - x).min(y.min(6 - y));
                if edge != 1 {
                    for j in 0..4 {
                        for i in 0..4 {
                            pixels[(16 + y * 4 + j) * 240 + 10 + k * 44 + x * 4 + i] = 0;
                        }
                    }
                }
            }
        }
    }
    let mut cfg = DefaultConfig {
        max_finders: 3,
        ..Default::default()
    };
    assert_eq!(run(&pixels, 240, 60, 240, &cfg), Err(Error::FinderCapacity));
    cfg.max_finders = 8;
    cfg.max_candidates = 1;
    assert_eq!(
        run(&pixels, 240, 60, 240, &cfg),
        Err(Error::CandidateBudget)
    );
    let file = include_bytes!("../../../fixtures/perspective-strong-v1.pgm");
    let offset = file
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(2)
        .unwrap()
        .0
        + 1;
    let cfg = DefaultConfig {
        max_retries: 1,
        ..Default::default()
    };
    assert_eq!(
        run(&file[offset..], 360, 360, 360, &cfg),
        Err(Error::RetryBudget)
    );
}
#[test]
fn bounded_noise_on_a_symbol() {
    let (mut pixels, w) = image(7, 4);
    let mut rng = 123u32;
    for p in &mut pixels {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        if rng % 1000 == 0 {
            *p = 255 - *p;
        } else {
            let noise = ((rng >> 24) % 20) as u8;
            *p = if *p == 0 { noise } else { 255 - noise };
        }
    }
    run(&pixels, w, w, w, &DefaultConfig::default()).unwrap();
}

#[test]
fn anisotropic_module_pitch() {
    for (sx, sy) in [(4, 4), (2, 4), (4, 2)] {
        let (pixels, w, h) = image_rect(7, sx, sy);
        let found = run(&pixels, w, h, w, &DefaultConfig::default())
            .unwrap_or_else(|e| panic!("{sx}x{sy}: {e:?}"));
        assert_eq!(found.decoded.version, 7);
    }
}

fn undecodable_frame() -> Vec<u8> {
    let bits = include_bytes!("../../../fixtures/v7.bin");
    let matrix = qr_core::Matrix::new(bits, 45).unwrap();
    let mut pixels = vec![255; 640 * 640];
    let mut rng = 0x12345678u32;
    for y in 0..45 {
        for x in 0..45 {
            let mut dark = matrix.get(x, y) == Some(true);
            if !qr_core::is_function(7, x, y) {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                dark = rng & 0x8000_0000 != 0;
            }
            if dark {
                for j in 0..8 {
                    for i in 0..8 {
                        pixels[(140 + y * 8 + j) * 640 + 140 + x * 8 + i] = 0;
                    }
                }
            }
        }
    }
    pixels
}

fn decode_stats(
    pixels: &[u8],
    config: &DefaultConfig,
    output: &mut [u8],
    segments: &mut [Segment],
) -> (Result<Detection, Error>, Stats) {
    let mut scratch = vec![0; config.scratch_bytes().unwrap()];
    let mut core_scratch = vec![0; SCRATCH_BYTES];
    // Every call must reset the counters, even when it stops before scanning.
    let mut stats = Stats {
        rows: 9,
        finder_observations: 9,
        candidates: 9,
        retries: 9,
        passes: 9,
    };
    let result = decode_with_stats(
        GrayImage::new(pixels, 640, 640, 640).unwrap(),
        config,
        &mut scratch,
        &mut core_scratch,
        output,
        segments,
        &mut stats,
    );
    if let Ok(found) = result {
        assert_eq!(found.stats, stats);
    }
    (result, stats)
}

#[test]
fn shared_pass_budgets_and_final_error_priority_keep_stats() {
    let pixels = undecodable_frame();
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for (max_candidates, max_retries, error, candidates, passes) in [
        (4096, 256, Error::Decode(qr_core::Error::Version), 2, 4),
        (1, 256, Error::CandidateBudget, 1, 3),
        (4096, 30, Error::RetryBudget, 2, 3),
    ] {
        let config = DefaultConfig {
            max_candidates,
            max_retries,
            ..Default::default()
        };
        let (result, stats) = decode_stats(&pixels, &config, &mut output, &mut segments);
        assert_eq!(result, Err(error));
        assert_eq!(
            stats,
            Stats {
                rows: 640 * passes,
                finder_observations: if passes == 4 { 304 } else { 264 },
                candidates,
                retries: if passes == 4 { 60 } else { 30 },
                passes,
            }
        );
    }
    // This extra finder is scanned after the symbol's three finders. Overflow
    // must take precedence even when the retained triple produces decode errors.
    let mut pixels = pixels;
    for y in 0..7 {
        for x in 0..7 {
            let edge = x.min(6 - x).min(y.min(6 - y));
            if edge != 1 {
                for j in 0..8 {
                    for i in 0..8 {
                        pixels[(560 + y * 8 + j) * 640 + 40 + x * 8 + i] = 0;
                    }
                }
            }
        }
    }
    let config = DefaultConfig {
        max_finders: 3,
        ..Default::default()
    };
    let (result, stats) = decode_stats(&pixels, &config, &mut output, &mut segments);
    assert_eq!(result, Err(Error::FinderCapacity));
    assert_eq!(
        stats,
        Stats {
            rows: 2560,
            finder_observations: 352,
            candidates: 2,
            retries: 60,
            passes: 4
        }
    );
    let (result, stats) = decode_stats(
        &vec![255; 640 * 640],
        &DefaultConfig::default(),
        &mut output,
        &mut segments,
    );
    assert_eq!(result, Err(Error::NotFound));
    assert_eq!(stats, Stats::default());
}

#[test]
fn capacity_errors_stop_on_first_decoded_matrix() {
    let (symbol, width) = image(7, 8);
    let mut pixels = vec![255; 640 * 640];
    for y in 0..width {
        pixels[(108 + y) * 640 + 108..(108 + y) * 640 + 108 + width]
            .copy_from_slice(&symbol[y * width..(y + 1) * width]);
    }
    let mut output = [0; 32];
    let mut segments = [Segment::EMPTY; 8];
    for (out_capacity, segment_capacity, error) in [
        (0, 8, qr_core::Error::OutputTooSmall),
        (32, 0, qr_core::Error::SegmentsTooSmall),
    ] {
        let (result, stats) = decode_stats(
            &pixels,
            &DefaultConfig::default(),
            &mut output[..out_capacity],
            &mut segments[..segment_capacity],
        );
        assert_eq!(result, Err(Error::Decode(error)));
        assert_eq!(
            stats,
            Stats {
                rows: 640,
                finder_observations: 128,
                candidates: 1,
                retries: 1,
                passes: 1
            }
        );
    }
    let (result, stats) = decode_stats(
        &pixels,
        &DefaultConfig::default(),
        &mut output,
        &mut segments,
    );
    assert!(result.is_ok());
    assert_eq!(
        stats,
        Stats {
            rows: 640,
            finder_observations: 128,
            candidates: 1,
            retries: 1,
            passes: 1
        }
    );
}
