use super::*;
use crate::{qr_default_config, qr_memory, QrMemory};
struct ModeCall {
    config: QrConfig,
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    detector: Vec<u8>,
    core: Vec<u8>,
    payload: [u8; 32],
    segments: [QrSegment; 8],
    result: QrResult,
    stats: QrStats,
    tracker: QrTracker,
    cache: Vec<u8>,
}
impl ModeCall {
    fn new(file: &[u8]) -> Self {
        let mut lines = file.splitn(4, |&b| b == b'\n');
        assert_eq!(lines.next().unwrap(), b"P5");
        let dimensions = core::str::from_utf8(lines.next().unwrap()).unwrap();
        let mut dimensions = dimensions.split_whitespace().map(|n| n.parse().unwrap());
        let width = dimensions.next().unwrap();
        let height = dimensions.next().unwrap();
        assert_eq!(lines.next().unwrap(), b"255");
        let pixels = lines.next().unwrap().to_vec();
        let mut config = QrConfig::default();
        let mut memory = QrMemory::default();
        unsafe {
            assert_eq!(qr_default_config(&mut config), 0);
            assert_eq!(qr_memory(&config, &mut memory), 0);
        }
        Self {
            config,
            pixels,
            width,
            height,
            detector: vec![0xa5; memory.detector_scratch_bytes],
            core: vec![0xa5; memory.core_scratch_bytes],
            payload: [0xa5; 32],
            segments: [QrSegment::default(); 8],
            result: QrResult {
                version: 777,
                ..QrResult::default()
            },
            stats: QrStats {
                pixel_reads: 777,
                ..QrStats::default()
            },
            tracker: QrTracker::default(),
            cache: vec![0xa5; width * 5],
        }
    }
    fn args(&mut self, mode: u32, strategy: u32, budget: usize) -> GrayCall {
        GrayCall {
            cache: None,
            c: &self.config,
            max_pixel_reads: budget,
            strategy,
            mode,
            tracker: Some(&mut self.tracker),
            pixels: self.pixels.as_ptr(),
            pixels_len: self.pixels.len(),
            width: self.width,
            height: self.height,
            stride: self.width,
            detector_scratch: self.detector.as_mut_ptr(),
            detector_scratch_len: self.detector.len(),
            core_scratch: self.core.as_mut_ptr(),
            core_scratch_len: self.core.len(),
            payload: self.payload.as_mut_ptr(),
            payload_len: self.payload.len(),
            segments: self.segments.as_mut_ptr(),
            segment_capacity: self.segments.len(),
            out: &mut self.result,
            stats: &mut self.stats,
        }
    }
    fn call(&mut self, api: u8, mode: u32, strategy: u32, budget: usize) -> i32 {
        let a = self.args(mode, strategy, budget);
        macro_rules! call {
            ($function:ident, [$($prefix:expr),*], [$($suffix:expr),*]) => {
                unsafe { $function($($prefix,)* a.pixels, a.pixels_len, a.width,
                    a.height, a.stride, a.detector_scratch, a.detector_scratch_len,
                    a.core_scratch, a.core_scratch_len, a.payload, a.payload_len,
                    a.segments, a.segment_capacity, a.out $(, $suffix)*) }
            };
        }
        match api {
            0 => call!(qr_decode_gray, [a.c], []),
            1 => call!(qr_decode_gray_with_budget, [a.c, budget], []),
            2 => call!(
                qr_decode_gray_with_sampling,
                [a.c, budget, strategy],
                [a.stats]
            ),
            3 => call!(
                qr_tracker_decode_gray,
                [a.tracker.unwrap(), a.c, budget, strategy],
                [a.stats]
            ),
            4 => call!(
                qr_decode_gray_with_sampling_cached,
                [a.c, budget, strategy],
                [a.stats, self.cache.as_mut_ptr(), self.cache.len()]
            ),
            5 => call!(
                qr_tracker_decode_gray_cached,
                [a.tracker.unwrap(), a.c, budget, strategy],
                [a.stats, self.cache.as_mut_ptr(), self.cache.len()]
            ),
            6 => call!(
                qr_decode_gray_with_mode,
                [a.c, budget, mode, strategy],
                [a.stats]
            ),
            7 => call!(
                qr_tracker_decode_gray_with_mode,
                [a.tracker.unwrap(), a.c, budget, mode, strategy],
                [a.stats]
            ),
            _ => unreachable!(),
        }
    }
    fn snapshot(&self) -> String {
        format!(
            "{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
            self.detector,
            self.core,
            self.payload,
            self.segments,
            self.result,
            self.stats,
            self.tracker
        )
    }
}
#[test]
fn mode_entry_points_decode_fixtures_with_both_sampling_strategies() {
    for file in [
        include_bytes!("../../../../fixtures/v1.pgm").as_slice(),
        include_bytes!("../../../../fixtures/lighting-v2.pgm").as_slice(),
        include_bytes!("../../../../fixtures/perspective-v7.pgm").as_slice(),
    ] {
        for mode in [QR_MODE_DIRECT, QR_MODE_FAST] {
            for strategy in [QR_SAMPLING_CENTER, QR_SAMPLING_CENTER_THEN_MAJORITY5] {
                for api in [6, 7] {
                    let mut case = ModeCall::new(file);
                    assert_eq!(
                        case.call(api, mode, strategy, 64_000_000),
                        0,
                        "mode={mode}, strategy={strategy}, api={api}"
                    );
                    assert_eq!(&case.payload[..case.result.payload_len], b"TEST");
                    assert!(case.stats.pixel_reads <= 64_000_000);
                    assert!(case.stats.passes <= if api == 7 { 16 } else { 8 });
                    if api == 7 {
                        assert_eq!(case.tracker.tag, 2);
                        // Exercise retained ROI state, using the same mode.
                        assert_eq!(case.call(api, mode, strategy, 64_000_000), 0);
                        assert_eq!(&case.payload[..case.result.payload_len], b"TEST");
                    }
                }
            }
        }
    }
}
#[test]
fn unknown_modes_preserve_all_storage_before_decoder_invocation() {
    for api in [6, 7] {
        let mut case = ModeCall::new(include_bytes!("../../../../fixtures/v1.pgm"));
        assert_eq!(case.call(7, QR_MODE_DIRECT, 0, 64_000_000), 0);
        let before = case.snapshot();
        for invalid_mode in [1, u32::MAX] {
            assert_eq!(case.call(api, invalid_mode, 0, 64_000_000), 12);
            assert_eq!(case.snapshot(), before);
        }
    }
}
#[test]
fn explicit_modes_preserve_overlap_and_terminal_budget_contracts() {
    for mode in [QR_MODE_DIRECT, QR_MODE_FAST] {
        let mut case = ModeCall::new(include_bytes!("../../../../fixtures/v1.pgm"));
        assert_eq!(case.call(7, mode, 0, 64_000_000), 0);
        let tracker = format!("{:?}", case.tracker);
        let result = format!("{:?}", case.result);
        let args = case.args(mode, 0, 1);
        let before = case.snapshot();
        assert_eq!(
            unsafe {
                gray_call(GrayCall {
                    payload: args.pixels.cast_mut(),
                    ..args
                })
            },
            2
        );
        assert_eq!(case.snapshot(), before);
        assert_eq!(case.call(7, mode, 0, 1), 18);
        assert!(case.stats.pixel_reads <= 1);
        assert_eq!(format!("{:?}", case.tracker), tracker);
        assert_eq!(format!("{:?}", case.result), result);
    }
}
#[test]
fn mode_unspecified_entry_points_use_direct_and_cached_apis_preserve_results() {
    assert_eq!(QR_DEFAULT_DETECTION_MODE, QR_MODE_DIRECT);
    for file in [
        include_bytes!("../../../../fixtures/v1.pgm").as_slice(),
        include_bytes!("../../../../fixtures/lighting-v2.pgm").as_slice(),
    ] {
        for api in 0..6 {
            let mut reference = ModeCall::new(file);
            let expected_mode = QR_MODE_DIRECT;
            assert_eq!(
                reference.call(6, expected_mode, QR_DEFAULT_SAMPLING_STRATEGY, 64_000_000),
                0
            );
            let mut case = ModeCall::new(file);
            assert_eq!(
                case.call(api, u32::MAX, QR_DEFAULT_SAMPLING_STRATEGY, 64_000_000),
                0
            );
            assert_eq!(
                format!("{:?}", case.result),
                format!("{:?}", reference.result)
            );
            assert_eq!(case.payload, reference.payload);
            assert_eq!(
                format!("{:?}", case.segments),
                format!("{:?}", reference.segments)
            );
            if api == 2 || api == 3 {
                assert_eq!(
                    format!("{:?}", case.stats),
                    format!("{:?}", reference.stats)
                );
            }
        }
    }
}
#[test]
fn cached_c_contracts_query_overlap_capacity_stats_and_tail() {
    let cfg = QrConfig {
        max_finders: 32,
        max_candidates: 4096,
        max_retries: 256,
        row_step: 1,
        local_radius: 24,
    };
    let mut needed = 777usize;
    unsafe {
        assert_eq!(qr_local_cache_memory(&cfg, 8, &mut needed), 0);
        assert_eq!(needed, 40);
        assert_eq!(qr_local_cache_memory(&cfg, 0, &mut needed), 12);
        assert_eq!(qr_local_cache_memory(&cfg, MAX_WIDTH + 1, &mut needed), 12);
        assert_eq!(qr_local_cache_memory(&cfg, 8, ptr::null_mut()), 1);
        assert_eq!(qr_local_cache_memory(ptr::null(), 8, &mut needed), 1);
        assert_eq!(
            qr_local_cache_memory((&needed as *const usize).cast(), 8, &mut needed),
            2
        );
    }
    let pixels: [u8; 64] = core::array::from_fn(|i| if i % 2 == 0 { 0 } else { 255 });
    let mut ds = [0; 4557];
    let mut cs = [0; qcore::SCRATCH_BYTES];
    let mut output = [0; 32];
    let mut segments = [QrSegment::default(); 8];
    let mut out = QrResult::default();
    let mut stats = QrStats::default();
    let mut tracker = QrTracker::default();
    let mut cache = [0xa5; 48];
    let args = GrayCall {
        cache: Some((cache.as_mut_ptr(), cache.len())),
        mode: QR_DEFAULT_DETECTION_MODE,
        c: &cfg,
        max_pixel_reads: 201,
        strategy: 0,
        tracker: Some(&mut tracker),
        pixels: pixels.as_ptr(),
        pixels_len: pixels.len(),
        width: 8,
        height: 8,
        stride: 8,
        detector_scratch: ds.as_mut_ptr(),
        detector_scratch_len: ds.len(),
        core_scratch: cs.as_mut_ptr(),
        core_scratch_len: cs.len(),
        payload: output.as_mut_ptr(),
        payload_len: output.len(),
        segments: segments.as_mut_ptr(),
        segment_capacity: segments.len(),
        out: &mut out,
        stats: &mut stats,
    };
    // All cache aliases rejected before any writable slice/initialization.
    let overlaps = [
        args.c.cast_mut().cast::<u8>(),
        args.pixels.cast_mut(),
        args.detector_scratch,
        args.core_scratch,
        args.payload,
        args.segments.cast(),
        args.out.cast(),
        args.stats.cast(),
        args.tracker.unwrap().cast(),
    ];
    for p in overlaps {
        assert_eq!(
            unsafe {
                gray_call(GrayCall {
                    cache: Some((p, 40)),
                    ..args
                })
            },
            2
        );
    }
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                cache: Some((ptr::null_mut(), 40)),
                ..args
            })
        },
        1
    );
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                cache: Some(((usize::MAX - 3) as *mut u8, 40)),
                ..args
            })
        },
        1
    );
    stats.pixel_reads = 777;
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                cache: Some((cache.as_mut_ptr(), 39)),
                ..args
            })
        },
        13
    );
    assert_eq!(stats.pixel_reads, 777);
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                cache: Some((ptr::null_mut(), 0)),
                ..args
            })
        },
        13
    );
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                cache: Some((cache.as_mut_ptr(), 40)),
                ..args
            })
        },
        18
    );
    assert_eq!(stats.pixel_reads, 201);
    assert_eq!(tracker.tag, 0);
    assert!(cache[40..].iter().all(|&v| v == 0xa5));
    assert_eq!(unsafe { gray_call(args) }, 18);
    assert!(cache[40..].iter().all(|&v| v == 0xa5));
    assert_eq!(
        unsafe {
            gray_call(GrayCall {
                stats: ptr::null_mut(),
                ..args
            })
        },
        18
    );
}
#[test]
fn integer_tracker_roundtrips_and_rejects_noncanonical_fields() {
    let hint = detect::TrackHint {
        roi: detect::Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 80,
        },
        image_width: 400,
        image_height: 300,
    };
    for state in [
        detect::TrackerState::Searching,
        detect::TrackerState::Candidate { hint, misses: 1 },
        detect::TrackerState::Tracking { hint },
    ] {
        assert_eq!(restore(save(state)).unwrap().state(), state);
    }
    let valid = save(detect::TrackerState::Tracking { hint });
    let mut invalid = [valid; 7];
    invalid[0].tag = 3;
    invalid[1].misses = 1;
    invalid[2].roi_x = usize::MAX;
    invalid[3].roi_height = 0;
    invalid[4].image_width = MAX_WIDTH + 1;
    invalid[5].image_height = 0;
    invalid[6].roi_y = 300;
    for t in invalid {
        assert!(restore(t).is_err());
    }
    assert!(restore(QrTracker {
        roi_x: 1,
        ..QrTracker::default()
    })
    .is_err());
}
