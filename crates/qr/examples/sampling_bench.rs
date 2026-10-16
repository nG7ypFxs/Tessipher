//! Paired sampling-policy and tracker benchmark; allocation/loading/printing are untimed.
use std::{hint::black_box, time::Instant};
use tessipher::{core, detect::*};

#[derive(Clone)]
struct Frame {
    name: String,
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}
struct Buffers {
    ds: Vec<u8>,
    cs: Vec<u8>,
    output: [u8; 32],
    segments: [core::Segment; 8],
    decoded: Option<core::Decoded>,
}
impl Buffers {
    fn new(config: &DefaultConfig) -> Self {
        Self {
            ds: vec![0; config.scratch_bytes().unwrap()],
            cs: vec![0; core::SCRATCH_BYTES],
            output: [0; 32],
            segments: [core::Segment::EMPTY; 8],
            decoded: None,
        }
    }
    fn run(
        &mut self,
        frame: &Frame,
        config: &DefaultConfig,
        strategy: SamplingStrategy,
        tracker: Option<&mut Tracker>,
    ) -> (i32, Stats, u128) {
        let image = GrayImage::new(&frame.pixels, frame.width, frame.height, frame.width).unwrap();
        let mut stats = Stats::default();
        let start = Instant::now();
        let result = black_box(match tracker {
            Some(t) => t.decode_with_stats(
                image,
                config,
                strategy,
                &mut self.ds,
                &mut self.cs,
                &mut self.output,
                &mut self.segments,
                &mut stats,
            ),
            None => decode_with_sampling(
                image,
                config,
                strategy,
                &mut self.ds,
                &mut self.cs,
                &mut self.output,
                &mut self.segments,
                &mut stats,
            ),
        });
        let nanos = start.elapsed().as_nanos();
        self.decoded = None;
        let outcome = match result {
            Ok(d) => {
                self.decoded = Some(d.decoded);
                if !frame.name.ends_with(".pgm") {
                    assert!(
                        &self.output[..d.decoded.payload_len] == b"TEST",
                        "public fixture payload mismatch"
                    );
                }
                0
            }
            Err(Error::Decode(e)) => 100 + e as i32,
            Err(Error::NotFound) => 17,
            Err(Error::PixelBudget) => 18,
            Err(Error::RetryBudget) => 16,
            Err(Error::CandidateBudget) => 15,
            Err(Error::FinderCapacity) => 14,
            Err(_) => 1,
        };
        (outcome, stats, nanos)
    }
}
fn bits(version: u8) -> &'static [u8] {
    match version {
        1 => include_bytes!("../../../fixtures/v1.bin"),
        7 => include_bytes!("../../../fixtures/v7.bin"),
        32 => include_bytes!("../../../fixtures/v32.bin"),
        40 => include_bytes!("../../../fixtures/v40.bin"),
        _ => unreachable!(),
    }
}
fn render(
    name: &str,
    version: u8,
    sx: usize,
    sy: usize,
    dims: (usize, usize),
    origin: (usize, usize),
    damage: u8,
) -> Frame {
    let (width, height) = dims;
    let (left, top) = origin;
    let side = 17 + version as usize * 4;
    let matrix = core::Matrix::new(bits(version), side).unwrap();
    let mut pixels = vec![255; width * height];
    let mut rng = 0x12345678u32;
    for y in 0..side {
        for x in 0..side {
            let mut dark = matrix.get(x, y).unwrap();
            if damage == 2 && !core::is_function(version, x, y) {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                dark = rng & 0x8000_0000 != 0;
            }
            for dy in 0..sy {
                for dx in 0..sx {
                    pixels[(top + y * sy + dy) * width + left + x * sx + dx] =
                        if dark { 0 } else { 255 };
                }
            }
            if damage == 1 && !core::is_function(version, x, y) {
                for dy in sy / 2 - 1..=sy / 2 {
                    for dx in sx / 2 - 1..=sx / 2 {
                        let i = (top + y * sy + dy) * width + left + x * sx + dx;
                        pixels[i] = 255 - pixels[i];
                    }
                }
            }
        }
    }
    Frame {
        name: name.into(),
        pixels,
        width,
        height,
    }
}
fn pgm(name: &str, data: &[u8]) -> Frame {
    let end = data
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(2)
        .unwrap()
        .0
        + 1;
    let header = std::str::from_utf8(&data[..end]).unwrap();
    let mut words = header.split_whitespace();
    words.next();
    let width = words.next().unwrap().parse().unwrap();
    let height = words.next().unwrap().parse().unwrap();
    Frame {
        name: name.into(),
        pixels: data[end..].to_vec(),
        width,
        height,
    }
}
fn corpus() -> Vec<Frame> {
    let normal = render("normal-v1", 1, 8, 8, (640, 640), (140, 140), 0);
    let mut mirror = normal.clone();
    mirror.name = "mirror-v1".into();
    for row in mirror.pixels.chunks_exact_mut(640) {
        row.reverse();
    }
    let mut noise = Frame {
        name: "fixed-noise".into(),
        pixels: vec![0; 640 * 640],
        width: 640,
        height: 640,
    };
    let mut rng = 0x12345678u32;
    for p in &mut noise.pixels {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        *p = (rng >> 24) as u8;
    }
    vec![
        normal,
        render("normal-v7", 7, 8, 8, (640, 640), (140, 140), 0),
        render("normal-v32", 32, 3, 3, (480, 480), (20, 20), 0),
        render("normal-v40", 40, 3, 3, (600, 600), (30, 30), 0),
        mirror,
        pgm(
            "rotation45",
            include_bytes!("../../../fixtures/rotated45-v2.pgm"),
        ),
        pgm(
            "perspective",
            include_bytes!("../../../fixtures/perspective-strong-v1.pgm"),
        ),
        pgm(
            "lighting",
            include_bytes!("../../../fixtures/lighting-v2.pgm"),
        ),
        render("low-resolution", 1, 1, 1, (64, 64), (16, 16), 0),
        render("anisotropic", 1, 4, 7, (200, 240), (40, 40), 0),
        noise,
        Frame {
            name: "no-qr".into(),
            pixels: vec![255; 640 * 640],
            width: 640,
            height: 640,
        },
        render("undecodable-v7", 7, 8, 8, (640, 640), (140, 140), 2),
        render("center-damage", 1, 8, 8, (240, 240), (32, 32), 1),
    ]
}
fn distribution(mut times: [u128; 15]) -> (f64, f64, f64) {
    times.sort();
    (
        times[0] as f64 / 1000.0,
        times[7] as f64 / 1000.0,
        times[14] as f64 / 1000.0,
    )
}
fn ratio(a: [u128; 15], b: [u128; 15]) -> f64 {
    let mut r = [0.0; 15];
    for i in 0..15 {
        r[i] = b[i] as f64 / a[i].max(1) as f64;
    }
    r.sort_by(f64::total_cmp);
    r[7]
}
// Explicit policy/frame/sample indices preserve the paired measurement columns.
#[allow(clippy::needless_range_loop)]
fn main() {
    if cfg!(debug_assertions) {
        eprintln!("run with --release");
        return;
    }
    // Gate fixed before timing: preserve old successes/work, <=10% normal paired
    // time growth, <=50% representative long-failure time/read growth, and no
    // old core-content failure changing into a budget failure under default limits.
    println!("gate=success_preserved,center_success_work_same,normal_paired_ratio<=1.10,long_failure_time_and_reads<=1.50,no_content_to_budget; host={} {}; warmup=3 samples=15 alternating_pair_order seed=0x12345678", std::env::consts::ARCH,std::env::consts::OS);
    let config = DefaultConfig::default();
    let mut buffers = [Buffers::new(&config), Buffers::new(&config)];
    let strategies = [
        SamplingStrategy::Center,
        SamplingStrategy::CenterThenMajority5,
    ];
    let mut gate = true;
    let mut frames = corpus();
    // Optional temporary PGM inputs are never copied to repository artifacts.
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        let name = std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        frames.push(pgm(name, &bytes));
    }
    for frame in &frames {
        for _ in 0..3 {
            for s in 0..2 {
                buffers[s].run(frame, &config, strategies[s], None);
            }
        }
        let mut times = [[0; 15]; 2];
        let mut outcomes = [[0; 15]; 2];
        let mut work = [Stats::default(); 2];
        for i in 0..15 {
            for s in [i % 2, 1 - i % 2] {
                let (out, stats, time) = buffers[s].run(frame, &config, strategies[s], None);
                if i > 0 {
                    assert_eq!(stats, work[s], "nondeterministic work");
                    assert_eq!(out, outcomes[s][0], "nondeterministic outcome");
                }
                times[s][i] = time;
                outcomes[s][i] = out;
                work[s] = stats;
            }
        }
        if let (Some(a), Some(b)) = (buffers[0].decoded, buffers[1].decoded) {
            assert!(
                a.version == b.version
                    && a.ec_level == b.ec_level
                    && a.mask == b.mask
                    && a.fnc1 == b.fnc1
                    && a.structured_append == b.structured_append
                    && a.payload_len == b.payload_len
                    && a.segment_count == b.segment_count,
                "successful metadata mismatch"
            );
            assert!(
                buffers[0].output[..a.payload_len] == buffers[1].output[..b.payload_len],
                "successful payload mismatch"
            );
            assert!(
                buffers[0].segments[..a.segment_count] == buffers[1].segments[..b.segment_count],
                "successful segments mismatch"
            );
        }
        let paired = ratio(times[0], times[1]);
        let success_ok = outcomes[0][0] != 0 || outcomes[1][0] == 0;
        let work_ok = outcomes[0][0] != 0 || work[0].retries != 1 || work[0] == work[1];
        let normal_time_ok = outcomes[0][0] != 0 || paired <= 1.10;
        let failure_ok = frame.name != "undecodable-v7"
            || (paired <= 1.50 && work[1].pixel_reads as f64 / work[0].pixel_reads as f64 <= 1.50);
        let budget_ok =
            !(matches!(outcomes[0][0], 103..=108) && matches!(outcomes[1][0], 15 | 16 | 18));
        gate &= success_ok && work_ok && normal_time_ok && failure_ok && budget_ok;
        println!("case={} center_us={:?} majority_us={:?} paired_ratio={:.4} gate_checks={:?}\n center_outcomes={:?} work={:?}\n majority_outcomes={:?} work={:?}",frame.name,distribution(times[0]),distribution(times[1]),paired,(success_ok,work_ok,normal_time_ok,failure_ok,budget_ok),outcomes[0],work[0],outcomes[1],work[1]);
    }
    for frame in frames.iter().filter(|f| f.name.ends_with(".pgm")) {
        for _ in 0..3 {
            for s in 0..2 {
                let mut tracker = Tracker::new();
                for _ in 0..2 {
                    buffers[s].run(frame, &config, strategies[s], Some(&mut tracker));
                }
            }
        }
        let mut times = [[[0; 15]; 2]; 2];
        let mut work = [[Stats::default(); 2]; 2];
        for i in 0..15 {
            for s in [i % 2, 1 - i % 2] {
                let mut tracker = Tracker::new();
                for n in 0..2 {
                    let (_, stats, time) =
                        buffers[s].run(frame, &config, strategies[s], Some(&mut tracker));
                    times[n][s][i] = time;
                    work[n][s] = stats;
                }
            }
        }
        for n in 0..2 {
            println!("private_stable case={} frame={} center_us={:?} majority_us={:?} paired_ratio={:.4} work={:?}", frame.name,n,distribution(times[n][0]),distribution(times[n][1]),ratio(times[n][0],times[n][1]),work[n]);
        }
    }
    let sequence = vec![
        render("initial", 1, 4, 4, (400, 300), (40, 60), 0),
        render("stable", 1, 4, 4, (400, 300), (40, 60), 0),
        render("small-move", 1, 4, 4, (400, 300), (44, 64), 0),
        render("roi-outside-move", 1, 4, 4, (400, 300), (280, 180), 0),
        Frame {
            name: "disappear-first".into(),
            pixels: vec![255; 400 * 300],
            width: 400,
            height: 300,
        },
        Frame {
            name: "disappear-second".into(),
            pixels: vec![255; 400 * 300],
            width: 400,
            height: 300,
        },
        render("reappear", 1, 4, 4, (400, 300), (40, 60), 0),
        render("false-candidate-first", 1, 4, 4, (400, 300), (40, 60), 2),
        render("false-candidate-second", 1, 4, 4, (400, 300), (40, 60), 2),
        render("recovered", 1, 4, 4, (400, 300), (40, 60), 0),
        render("resize", 1, 4, 4, (320, 240), (160, 120), 0),
    ];
    for tracked in [false, true] {
        for _ in 0..3 {
            for s in 0..2 {
                let mut tracker = Tracker::new();
                for f in &sequence {
                    buffers[s].run(f, &config, strategies[s], tracked.then_some(&mut tracker));
                }
            }
        }
        let mut times = vec![[[0; 15]; 2]; sequence.len()];
        let mut totals = [[0; 15]; 2];
        let mut work = vec![[Stats::default(); 2]; sequence.len()];
        let mut outcomes = vec![[0; 2]; sequence.len()];
        for i in 0..15 {
            for s in [i % 2, 1 - i % 2] {
                let mut tracker = Tracker::new();
                for (n, f) in sequence.iter().enumerate() {
                    let (out, stats, time) =
                        buffers[s].run(f, &config, strategies[s], tracked.then_some(&mut tracker));
                    if i > 0 {
                        assert_eq!(stats, work[n][s], "sequence work mismatch");
                        assert_eq!(out, outcomes[n][s], "sequence outcome mismatch");
                    }
                    times[n][s][i] = time;
                    totals[s][i] += time;
                    work[n][s] = stats;
                    outcomes[n][s] = out;
                }
            }
        }
        println!("sequence tracked={tracked} center_total_us={:?} majority_total_us={:?} paired_ratio={:.4}",distribution(totals[0]),distribution(totals[1]),ratio(totals[0],totals[1]));
        for (n, f) in sequence.iter().enumerate() {
            println!(" frame={} center_us={:?} majority_us={:?} paired_ratio={:.4} center_outcomes={:?} majority_outcomes={:?} work={:?}",f.name,distribution(times[n][0]),distribution(times[n][1]),ratio(times[n][0],times[n][1]),[outcomes[n][0];15],[outcomes[n][1];15],work[n]);
        }
    }
    println!("DEFAULT_GATE_PASSED={gate}");
}
