#![cfg(feature = "micro-qr-detect")]

use qr_core::{Segment, SCRATCH_BYTES};
use qr_detect::{decode_micro, decode_micro_with_stats, Config, Error, GrayImage, Stats};

fn matrix(version: u8) -> (&'static [u8], usize, &'static [u8]) {
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    let mut cursor = 4;
    while cursor < corpus.len() {
        let current = corpus[cursor];
        let output_len = u16::from_le_bytes([corpus[cursor + 4], corpus[cursor + 5]]) as usize;
        let matrix_len = u16::from_le_bytes([corpus[cursor + 6], corpus[cursor + 7]]) as usize;
        cursor += 8;
        let output = &corpus[cursor..cursor + output_len];
        cursor += output_len;
        let bits = &corpus[cursor..cursor + matrix_len];
        cursor += matrix_len;
        if current == version {
            return (bits, version as usize * 2 + 9, output);
        }
    }
    panic!("missing M{version} fixture")
}

fn render(version: u8, scale_x: usize, scale_y: usize) -> (Vec<u8>, usize, usize, &'static [u8]) {
    let (bits, side, output) = matrix(version);
    let quiet = 2;
    let width = (side + quiet * 2) * scale_x;
    let height = (side + quiet * 2) * scale_y;
    let mut pixels = vec![255; width * height];
    for y in 0..side {
        for x in 0..side {
            let bit = y * side + x;
            if bits[bit / 8] & (0x80 >> (bit % 8)) == 0 {
                continue;
            }
            for dy in 0..scale_y {
                for dx in 0..scale_x {
                    let px = (x + quiet) * scale_x + dx;
                    let py = (y + quiet) * scale_y + dy;
                    pixels[py * width + px] = 0;
                }
            }
        }
    }
    (pixels, width, height, output)
}

fn run(pixels: &[u8], width: usize, height: usize) -> (u8, Vec<u8>) {
    let config = Config::<256, 256>::default();
    let image = GrayImage::new(pixels, width, height, width).unwrap();
    let mut detector = vec![0; config.scratch_bytes().unwrap()];
    let mut core = vec![0; SCRATCH_BYTES];
    let mut output = [0; 64];
    let mut segments = [Segment::EMPTY; 8];
    let found = decode_micro(
        image,
        &config,
        &mut detector,
        &mut core,
        &mut output,
        &mut segments,
    )
    .unwrap();
    (
        found.decoded.version as u8,
        output[..found.decoded.payload_len].to_vec(),
    )
}

fn rotate(pixels: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let mut result = vec![255; width * height];
    for y in 0..height {
        for x in 0..width {
            result[x * height + height - 1 - y] = pixels[y * width + x];
        }
    }
    (result, height, width)
}

fn rotate_45(pixels: &[u8], side: usize) -> (Vec<u8>, usize) {
    let output_side = (side as f32 * core::f32::consts::SQRT_2) as usize + 8;
    let mut result = vec![255; output_side * output_side];
    let source_center = (side as f32 - 1.0) * 0.5;
    let output_center = (output_side as f32 - 1.0) * 0.5;
    for y in 0..output_side {
        for x in 0..output_side {
            let dx = x as f32 - output_center;
            let dy = y as f32 - output_center;
            let sx = core::f32::consts::FRAC_1_SQRT_2 * (dx + dy) + source_center;
            let sy = core::f32::consts::FRAC_1_SQRT_2 * (-dx + dy) + source_center;
            if sx >= 0.0 && sy >= 0.0 && sx < side as f32 && sy < side as f32 {
                result[y * output_side + x] = pixels[sy as usize * side + sx as usize];
            }
        }
    }
    (result, output_side)
}

fn run_stats(
    pixels: &[u8],
    width: usize,
    height: usize,
    config: &Config<256, 256>,
    output: &mut [u8],
    segments: &mut [Segment],
) -> (Result<qr_detect::MicroDetection, Error>, Stats) {
    let image = GrayImage::new(pixels, width, height, width).unwrap();
    let mut detector = vec![0; config.scratch_bytes().unwrap()];
    let mut core = vec![0; SCRATCH_BYTES];
    let mut stats = Stats::default();
    let result = decode_micro_with_stats(
        image,
        config,
        &mut detector,
        &mut core,
        output,
        segments,
        &mut stats,
    );
    (result, stats)
}

#[test]
fn detects_m1_through_m4_inverted_and_anisotropic() {
    for version in 1..=4 {
        for (scale_x, scale_y) in [(4, 4), (3, 5), (5, 3)] {
            let (original, width, height, expected) = render(version, scale_x, scale_y);
            for inverted in [false, true] {
                let mut pixels: Vec<_> = original
                    .iter()
                    .map(|&value| if inverted { 255 - value } else { value })
                    .collect();
                let (mut rotated_width, mut rotated_height) = (width, height);
                for _ in 0..4 {
                    let (decoded_version, output) = run(&pixels, rotated_width, rotated_height);
                    assert_eq!(decoded_version, version);
                    assert_eq!(output, expected);
                    (pixels, rotated_width, rotated_height) =
                        rotate(&pixels, rotated_width, rotated_height);
                }
            }
        }
    }
}

#[test]
fn detector_respects_stride() {
    let (pixels, width, height, expected) = render(4, 4, 4);
    let stride = width + 13;
    let mut padded = vec![127; (height - 1) * stride + width];
    for y in 0..height {
        padded[y * stride..y * stride + width].copy_from_slice(&pixels[y * width..(y + 1) * width]);
    }
    let config = Config::<256, 256>::default();
    let image = GrayImage::new(&padded, width, height, stride).unwrap();
    let mut detector = vec![0; config.scratch_bytes().unwrap()];
    let mut core = vec![0; SCRATCH_BYTES];
    let mut output = [0; 64];
    let mut segments = [Segment::EMPTY; 8];
    let found = decode_micro(
        image,
        &config,
        &mut detector,
        &mut core,
        &mut output,
        &mut segments,
    )
    .unwrap();
    assert_eq!(&output[..found.decoded.payload_len], expected);
}

#[test]
fn detects_45_degree_reflection_and_bounded_noise() {
    let (mut pixels, width, _, expected) = render(4, 6, 6);
    for row in pixels.chunks_exact_mut(width) {
        row.reverse();
    }
    let (mut pixels, width) = rotate_45(&pixels, width);
    let mut state = 0x7421_9badu32;
    for pixel in &mut pixels {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = ((state >> 27) & 15) as u8;
        *pixel = if *pixel < 128 { noise } else { 255 - noise };
    }
    let (version, output) = run(&pixels, width, width);
    assert_eq!(version, 4);
    assert_eq!(output, expected);
}

#[test]
fn false_positives_and_budgets_are_deterministic() {
    let blank = [255; 64 * 64];
    let mut output = [0; 64];
    let mut segments = [Segment::EMPTY; 8];
    let config = Config::<256, 256>::default();
    let first = run_stats(&blank, 64, 64, &config, &mut output, &mut segments);
    let second = run_stats(&blank, 64, 64, &config, &mut output, &mut segments);
    assert_eq!(first, second);
    assert_eq!(first.0, Err(Error::NotFound));

    let mut finder_only = [255; 64 * 64];
    for y in 0..7 {
        for x in 0..7 {
            let edge = x == 0 || y == 0 || x == 6 || y == 6;
            let center = (2..=4).contains(&x) && (2..=4).contains(&y);
            if edge || center {
                for dy in 0..4 {
                    for dx in 0..4 {
                        finder_only[((y + 4) * 4 + dy) * 64 + (x + 4) * 4 + dx] = 0;
                    }
                }
            }
        }
    }
    let limited = Config::<256, 256> {
        max_candidates: 1,
        ..Default::default()
    };
    let (result, stats) = run_stats(&finder_only, 64, 64, &limited, &mut output, &mut segments);
    assert_eq!(result, Err(Error::CandidateBudget));
    assert_eq!(stats.candidates, 1);

    let (pixels, width, height, _) = render(4, 4, 4);
    let retry_limited = Config::<256, 256> {
        max_retries: 3,
        ..Default::default()
    };
    let (result, stats) = run_stats(
        &pixels,
        width,
        height,
        &retry_limited,
        &mut output,
        &mut segments,
    );
    assert_eq!(result, Err(Error::RetryBudget));
    assert_eq!(stats.retries, 3);

    let baseline = run_stats(&pixels, width, height, &config, &mut output, &mut segments);
    let reads = baseline.1.pixel_reads;
    assert!(baseline.0.is_ok());
    let pixel_limited = Config::<256, 256> {
        max_pixel_reads: reads - 1,
        ..Default::default()
    };
    let (result, stats) = run_stats(
        &pixels,
        width,
        height,
        &pixel_limited,
        &mut output,
        &mut segments,
    );
    assert_eq!(result, Err(Error::PixelBudget));
    assert_eq!(stats.pixel_reads, reads - 1);

    let (result, _) = run_stats(&pixels, width, height, &config, &mut [], &mut segments);
    assert_eq!(result, Err(Error::Decode(qr_core::Error::OutputTooSmall)));
    let (result, _) = run_stats(&pixels, width, height, &config, &mut output, &mut []);
    assert_eq!(result, Err(Error::Decode(qr_core::Error::SegmentsTooSmall)));
}
