#![cfg(all(feature = "detect", tessipher_repository_fixtures))]

use tessipher::{
    core::{Segment, SCRATCH_BYTES},
    detect::*,
};

// Render TEST matrices under spatially varying illumination. Cases include
// globally overlapping dark/light ranges, where extra global attempts must
// still fall back to the sampled local threshold.
fn render(bits: &[u8], side: usize, case: usize, inverted: bool) -> (Vec<u8>, usize) {
    let size = if case == 2 {
        480
    } else if case == 3 {
        420
    } else {
        360
    };
    let (x0, y0, extent) = if case == 2 {
        (72.0, 72.0, 336.0)
    } else if case == 3 {
        (67.0, 59.0, 294.0)
    } else {
        (50.0, 50.0, 250.0)
    };
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f64 + 0.5 - x0) * side as f64 / extent;
            let v = (y as f64 + 0.5 - y0) * side as f64 / extent;
            let dark = u >= 0.0 && v >= 0.0 && u < side as f64 && v < side as f64 && {
                let bit = v as usize * side + u as usize;
                bits[bit / 8] & (128 >> (bit % 8)) != 0
            };
            let value = match case {
                0 => (if dark { 35.0 } else { 130.0 }) + x as f64 * 0.30 + y as f64 * 0.04,
                1 => (if dark { 35.0 } else { 130.0 }) - x as f64 * 0.30 + y as f64 * 0.04 + 108.0,
                2 => (if dark { 20.0 } else { 105.0 }) + x as f64 * 0.31,
                _ => {
                    let nx = (x as f64 + 0.5) / size as f64;
                    let ny = (y as f64 + 0.5) / size as f64;
                    let shadow = 1.0
                        - 0.82
                            * (-0.5
                                * (((nx - 0.57) / 0.20).powi(2) + ((ny - 0.46) / 0.36).powi(2)))
                            .exp();
                    ((if dark { 40.0 } else { 205.0 }) + x as f64 * 0.02) * shadow
                }
            };
            let pixel = value.clamp(0.0, 255.0) as u8;
            pixels.push(if inverted { 255 - pixel } else { pixel });
        }
    }
    (pixels, size)
}

#[test]
fn lighting_gradients_and_shadows_preserve_payload_with_minimal_and_fast_scratch() {
    let config = DefaultConfig::default();
    let mut core = vec![0; SCRATCH_BYTES];
    let mut out = vec![0; 7089];
    let mut segments = [Segment::EMPTY; 64];
    for (version, bits) in [
        (1, include_bytes!("../../../fixtures/v1.bin").as_slice()),
        (2, include_bytes!("../../../fixtures/v2.bin").as_slice()),
        (7, include_bytes!("../../../fixtures/v7.bin").as_slice()),
    ] {
        for case in 0..4 {
            for inverted in [false, true] {
                let (pixels, width) = render(bits, 17 + version * 4, case, inverted);
                let image = GrayImage::new(&pixels, width, width, width).unwrap();
                for strategy in [
                    SamplingStrategy::Center,
                    SamplingStrategy::CenterThenMajority5,
                ] {
                    let mut reference = None;
                    for length in [
                        config.scratch_bytes().unwrap(),
                        config
                            .scratch_bytes_for_mode(DetectionMode::Fast, width, width)
                            .unwrap(),
                    ] {
                        let mut scratch = vec![0xff; length];
                        let mut stats = Stats::default();
                        let mut found = decode_with_mode(
                            image,
                            &config,
                            DetectionMode::Fast,
                            strategy,
                            &mut scratch,
                            &mut core,
                            &mut out,
                            &mut segments,
                            &mut stats,
                        )
                        .unwrap_or_else(|e| {
                            panic!("v{version} case{case} inverted{inverted}: {e:?}")
                        });
                        assert_eq!(&out[..found.decoded.payload_len], b"TEST");
                        assert!(stats.pixel_reads <= config.max_pixel_reads);
                        found.stats.pixel_reads = 0;
                        if let Some(old) = reference {
                            assert_eq!(found, old);
                        } else {
                            reference = Some(found);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn failed_global_refinement_leaves_sampling_budget_for_local_shadow_recovery() {
    let (pixels, width) = render(include_bytes!("../../../fixtures/v7.bin"), 45, 3, false);
    let image = GrayImage::new(&pixels, width, width, width).unwrap();
    let config = DefaultConfig {
        max_retries: 64,
        ..DefaultConfig::default()
    };
    let mut scratch = vec![
        0;
        config
            .scratch_bytes_for_mode(DetectionMode::Fast, width, width)
            .unwrap()
    ];
    let mut core = vec![0; SCRATCH_BYTES];
    let mut out = vec![0; 7089];
    let mut segments = [Segment::EMPTY; 64];
    let mut stats = Stats::default();
    let found = decode_with_mode(
        image,
        &config,
        DetectionMode::Fast,
        SamplingStrategy::Center,
        &mut scratch,
        &mut core,
        &mut out,
        &mut segments,
        &mut stats,
    )
    .unwrap();
    assert_eq!(&out[..found.decoded.payload_len], b"TEST");
    assert!(found.local_threshold);
    assert!(stats.retries <= 49);
    for limit in [41, 42] {
        let limited = DefaultConfig {
            max_retries: limit,
            ..config
        };
        let result = decode_with_mode(
            image,
            &limited,
            DetectionMode::Fast,
            SamplingStrategy::Center,
            &mut scratch,
            &mut core,
            &mut out,
            &mut segments,
            &mut stats,
        );
        assert_eq!(result, Err(Error::RetryBudget));
        assert_eq!(stats.retries, limit);
        assert_eq!(stats.passes, 3);
    }
}
