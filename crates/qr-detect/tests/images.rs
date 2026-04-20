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
