//! Deterministic mutation smoke harness; complements the libFuzzer targets under fuzz/.
use qr::{core, detect};
fn rand(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}
fn main() {
    let seed = 0x738a9015;
    let mut state = seed;
    let mut scratch = vec![0; core::SCRATCH_BYTES];
    let mut output = vec![0; core::MAX_PAYLOAD_BYTES];
    let mut segments = vec![core::Segment::EMPTY; 64];
    let mut counts = [0; 3];
    let start = std::time::Instant::now();
    for _ in 0..20000 {
        let n = (rand(&mut state) % 96) as usize;
        let mut data = vec![0; n];
        for b in &mut data {
            *b = rand(&mut state) as u8;
        }
        let version = (rand(&mut state) % 42) as u8;
        let out_len = (rand(&mut state) % 256) as usize;
        let seg_len = (rand(&mut state) % 64) as usize;
        let _ = core::parse_payload(
            &data,
            version,
            &mut output[..out_len],
            &mut segments[..seg_len],
        );
        counts[0] += 1;
    }
    let original = include_bytes!("../../../fixtures/v1.bin");
    for _ in 0..10000 {
        let mut bits = *original;
        let changes = (rand(&mut state) % 25) as usize;
        for _ in 0..changes {
            let bit = (rand(&mut state) % 441) as usize;
            bits[bit / 8] ^= 128 >> (bit % 8);
        }
        let m = core::Matrix::new(&bits, 21).unwrap();
        let _ = core::decode(m, &mut scratch, &mut output, &mut segments);
        counts[1] += 1;
    }
    let cfg = detect::Config::<96, 96> {
        max_finders: 8,
        max_candidates: 16,
        max_retries: 8,
        ..Default::default()
    };
    let mut ds = vec![0; cfg.scratch_bytes().unwrap()];
    for _ in 0..200 {
        let w = (rand(&mut state) % 96 + 1) as usize;
        let h = (rand(&mut state) % 96 + 1) as usize;
        let stride = w + (rand(&mut state) % 16) as usize;
        let mut pixels = vec![0; (h - 1) * stride + w];
        for b in &mut pixels {
            *b = rand(&mut state) as u8;
        }
        let image = detect::GrayImage::new(&pixels, w, h, stride).unwrap();
        let _ = detect::decode(
            image,
            &cfg,
            &mut ds,
            &mut scratch,
            &mut output,
            &mut segments,
        );
        counts[2] += 1;
    }
    println!(
        "seed={seed:#x}, payload={} matrix={} image={} elapsed={:.3}s",
        counts[0],
        counts[1],
        counts[2],
        start.elapsed().as_secs_f64()
    );
}
