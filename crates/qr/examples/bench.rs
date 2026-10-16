//! Host wall-clock distributions. On MCU measure the same call with a cycle counter.
use std::{hint::black_box, time::Instant};
use tessipher::{core, detect};
fn main() {
    let bits = include_bytes!("../../../fixtures/v7.bin");
    let matrix = core::Matrix::new(bits, 45).unwrap();
    let config = detect::DefaultConfig::default();
    let mut cs = vec![0; core::SCRATCH_BYTES];
    let mut ds = vec![0; config.scratch_bytes().unwrap()];
    let mut out = [0; 32];
    let mut segments = [core::Segment::EMPTY; 8];
    println!("host={} {} optimized={} samples=15 core_scratch={} detector_scratch={} align=1 config={config:?}",std::env::consts::ARCH,std::env::consts::OS,!cfg!(debug_assertions),cs.len(),ds.len());
    let mut times = [0.0f64; 15];
    for t in &mut times {
        let start = Instant::now();
        black_box(core::decode(matrix, &mut cs, &mut out, &mut segments).unwrap());
        *t = start.elapsed().as_secs_f64() * 1e6;
    }
    report("matrix-V7-H-mask3", times);
    for version in [1, 7, 32, 40] {
        println!(
            "matrix-V{version} required_core_scratch={}",
            core::scratch_bytes(version).unwrap()
        );
    }
    let small = core::Matrix::new(include_bytes!("../../../fixtures/v1.bin"), 21).unwrap();
    let mut minimal = vec![0; core::scratch_bytes(1).unwrap()];
    for t in &mut times {
        let start = Instant::now();
        black_box(core::decode(small, &mut minimal, &mut out, &mut segments).unwrap());
        *t = start.elapsed().as_secs_f64() * 1e6;
    }
    report("matrix-V1-minimal-scratch", times);
    // Higher versions exercise the larger alignment-center grids.
    for (version, bits) in [
        (32, include_bytes!("../../../fixtures/v32.bin").as_slice()),
        (40, include_bytes!("../../../fixtures/v40.bin").as_slice()),
    ] {
        let matrix = core::Matrix::new(bits, version * 4 + 17).unwrap();
        for t in &mut times {
            let start = Instant::now();
            black_box(
                core::decode(
                    matrix,
                    &mut cs[..core::scratch_bytes(matrix.version()).unwrap()],
                    &mut out,
                    &mut segments,
                )
                .unwrap(),
            );
            *t = start.elapsed().as_secs_f64() * 1e6;
        }
        report(&format!("matrix-V{version}"), times);
    }
    for case in ["normal", "no-QR", "noise", "undecodable"] {
        let mut pixels = vec![255; 640 * 640];
        let mut rng = 0x12345678u32;
        if case == "normal" || case == "undecodable" {
            for y in 0..45 {
                for x in 0..45 {
                    let mut dark = matrix.get(x, y) == Some(true);
                    if case == "undecodable" && !core::is_function(7, x, y) {
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
        }
        if case == "noise" {
            for p in &mut pixels {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                *p = (rng >> 24) as u8;
            }
        }
        let image = detect::GrayImage::new(&pixels, 640, 640, 640).unwrap();
        let mut stats = detect::Stats::default();
        let mut outcome = Ok(());
        for t in &mut times {
            let start = Instant::now();
            outcome = black_box(detect::decode_with_stats(
                image,
                &config,
                &mut ds,
                &mut cs,
                &mut out,
                &mut segments,
                &mut stats,
            ))
            .map(|_| ());
            *t = start.elapsed().as_secs_f64() * 1e6;
        }
        report(case, times);
        println!("  Gray8=640x640 stride=640 result={outcome:?} stats={stats:?}");
    }
}
fn report(name: &str, mut times: [f64; 15]) {
    times.sort_by(f64::total_cmp);
    println!(
        "{name}: min_us={:.2} median_us={:.2} test_set_max_us={:.2}",
        times[0], times[7], times[14]
    );
}
