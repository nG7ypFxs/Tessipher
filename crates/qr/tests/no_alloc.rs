#![cfg(feature = "detect")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Count;
thread_local! {
    // Constant initialization avoids allocating while the allocator accesses TLS.
    // None disables measurement; Some counts only this test thread's allocations.
    static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count_allocation() {
    // TLS may be unavailable during thread teardown. Allocators must not panic.
    let _ = ALLOCS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n.saturating_add(1)));
        }
    });
}

fn start_counting() {
    ALLOCS.with(|count| {
        assert!(count.get().is_none());
        count.set(Some(0));
    });
}

fn stop_counting() -> usize {
    ALLOCS.with(|count| {
        count
            .replace(None)
            .expect("allocation measurement is active")
    })
}

unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(p, l, new_size) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;

#[test]
fn counting_detects_allocations_and_resets() {
    let layout = Layout::from_size_align(16, 8).unwrap();
    start_counting();
    // Call the allocator directly so release optimization cannot elide the
    // allocation operations under the standard allocation API's guarantees.
    unsafe {
        let p = std::hint::black_box(ALLOCATOR.alloc(layout));
        assert!(!p.is_null());
        let resized = std::hint::black_box(ALLOCATOR.realloc(p, layout, 32));
        assert!(!resized.is_null());
        ALLOCATOR.dealloc(resized, Layout::from_size_align(32, 8).unwrap());
        let zeroed = std::hint::black_box(ALLOCATOR.alloc_zeroed(layout));
        assert!(!zeroed.is_null());
        ALLOCATOR.dealloc(zeroed, layout);
    }
    let allocations = stop_counting();
    assert_eq!(allocations, 3);

    start_counting();
    assert_eq!(stop_counting(), 0);
}

#[test]
fn counting_ignores_other_threads() {
    use std::sync::atomic::{AtomicBool, Ordering};

    // Spin instead of parking: synchronization must not allocate on the
    // measured thread, even when its thread parker is not initialized yet.
    let ready = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !ready.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            let layout = Layout::from_size_align(16, 8).unwrap();
            unsafe {
                let p = std::hint::black_box(ALLOCATOR.alloc(layout));
                assert!(!p.is_null());
                ALLOCATOR.dealloc(p, layout);
            }
            done.store(true, Ordering::Release);
        });
        start_counting();
        ready.store(true, Ordering::Release);
        while !done.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        let allocations = stop_counting();
        assert_eq!(allocations, 0);
    });
}

#[test]
fn decoding_does_not_allocate() {
    let bits = include_bytes!("../../../fixtures/v1.bin");
    let m = tessipher::Matrix::new(bits, 21).unwrap();
    let mut cs = vec![0; tessipher::core::SCRATCH_BYTES];
    let cfg = tessipher::DefaultConfig::default();
    let mut ds = vec![0; cfg.scratch_bytes().unwrap()];
    let mut out = [0; 16];
    let mut seg = [tessipher::Segment::EMPTY; 8];
    let pgm = include_bytes!("../../../fixtures/v1.pgm");
    let offset = pgm
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(2)
        .unwrap()
        .0
        + 1;
    let img = tessipher::GrayImage::new(&pgm[offset..], 145, 145, 145).unwrap();
    let mut mirrored_bits = [0u8; 56];
    for y in 0..21 {
        for x in 0..21 {
            if m.get(y, x) == Some(true) {
                let i = y * 21 + x;
                mirrored_bits[i / 8] |= 128 >> (i % 8);
            }
        }
    }
    let mirrored_matrix = tessipher::Matrix::new(&mirrored_bits, 21).unwrap();
    let mut mirrored_pixels = pgm[offset..].to_vec();
    for row in mirrored_pixels.chunks_exact_mut(145) {
        row.reverse();
    }
    let mirrored_image = tessipher::GrayImage::new(&mirrored_pixels, 145, 145, 145).unwrap();
    let mut damaged_pixels = pgm[offset..].to_vec();
    for y in 0..21 {
        for x in 0..21 {
            if !tessipher::core::is_function(1, x, y) {
                let i = ((y + 4) * 5 + 2) * 145 + (x + 4) * 5 + 2;
                damaged_pixels[i] = 255 - damaged_pixels[i];
            }
        }
    }
    let damaged_image = tessipher::GrayImage::new(&damaged_pixels, 145, 145, 145).unwrap();
    let mut stable_pixels = vec![255; 400 * 300];
    let mut moved_pixels = vec![255; 400 * 300];
    for y in 0..145 {
        stable_pixels[(y + 40) * 400 + 40..(y + 40) * 400 + 185]
            .copy_from_slice(&pgm[offset + y * 145..offset + (y + 1) * 145]);
        moved_pixels[(y + 140) * 400 + 220..(y + 140) * 400 + 365]
            .copy_from_slice(&pgm[offset + y * 145..offset + (y + 1) * 145]);
    }
    let stable_image = tessipher::GrayImage::new(&stable_pixels, 400, 300, 400).unwrap();
    let moved_image = tessipher::GrayImage::new(&moved_pixels, 400, 300, 400).unwrap();
    let blank_pixels = vec![255; 400 * 300];
    let blank_image = tessipher::GrayImage::new(&blank_pixels, 400, 300, 400).unwrap();
    let mut cache = [0; 5 * 640];
    let mut tracker = tessipher::Tracker::new();
    let mut stats = tessipher::Stats::default();
    start_counting();
    let a = tessipher::decode_matrix(m, &mut cs, &mut out, &mut seg);
    let b = tessipher::decode_gray(img, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    let c = tessipher::decode_matrix(mirrored_matrix, &mut cs, &mut out, &mut seg);
    let d = tessipher::decode_gray(mirrored_image, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    let e = tessipher::decode_gray_with_sampling(
        img,
        &cfg,
        tessipher::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let f = tracker.decode_with_stats(
        img,
        &cfg,
        tessipher::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let g = tracker.decode_with_stats(
        img,
        &cfg,
        tessipher::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let h = tessipher::decode_gray_with_sampling(
        damaged_image,
        &cfg,
        tessipher::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let i = tracker.decode_with_stats(
        stable_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    tracker = tessipher::Tracker::from_state(tracker.state()).unwrap();
    let j = tracker.decode_with_stats(
        stable_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let k = tracker.decode_with_stats(
        moved_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let l = tracker.decode_with_stats(
        blank_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let candidate = tracker.state();
    let n = tracker.decode_with_stats(
        blank_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let cached = tessipher::decode_gray_with_sampling_cached(
        img,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
        &mut cache,
    );
    let cached_tracked = tracker.decode_with_stats_cached(
        img,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
        &mut cache,
    );
    let cached_blank = tracker.decode_with_stats_cached(
        blank_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
        &mut cache,
    );
    let cached_blank_again = tracker.decode_with_stats_cached(
        blank_image,
        &cfg,
        tessipher::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
        &mut cache,
    );
    let allocations = stop_counting();
    assert!(
        a.is_ok()
            && b.is_ok()
            && c.is_ok()
            && d.is_ok()
            && e.is_ok()
            && f.is_ok()
            && g.is_ok()
            && h.is_ok()
    );
    assert!(i.is_ok() && j.is_ok() && k.is_ok());
    assert_eq!(l, Err(tessipher::detect::Error::NotFound));
    assert_eq!(n, Err(tessipher::detect::Error::NotFound));
    assert!(matches!(
        candidate,
        tessipher::TrackerState::Candidate { .. }
    ));
    assert_eq!(tracker.state(), tessipher::TrackerState::Searching);
    assert_eq!(h.unwrap().stats.retries, 2);
    assert!(cached.is_ok() && cached_tracked.is_ok());
    assert_eq!(cached_blank, Err(tessipher::detect::Error::NotFound));
    assert_eq!(cached_blank_again, Err(tessipher::detect::Error::NotFound));
    assert_eq!(allocations, 0);
}

#[cfg(feature = "micro-qr-detect")]
#[test]
fn micro_decoding_does_not_allocate() {
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    let output_len = u16::from_le_bytes([corpus[8], corpus[9]]) as usize;
    let matrix_len = u16::from_le_bytes([corpus[10], corpus[11]]) as usize;
    let bits = &corpus[12 + output_len..12 + output_len + matrix_len];
    let matrix = tessipher::MicroMatrix::new(bits, 11).unwrap();
    let mut core = [0; tessipher::core::MICRO_SCRATCH_BYTES];
    let config = tessipher::Config::<64, 64>::default();
    let mut detector = vec![0; config.scratch_bytes().unwrap()];
    let mut output = [0; 35];
    let mut segments = [tessipher::Segment::EMPTY; 4];
    let mut pixels = [255; 60 * 60];
    for y in 0..11 {
        for x in 0..11 {
            if matrix.get(x, y) != Some(true) {
                continue;
            }
            for dy in 0..4 {
                for dx in 0..4 {
                    pixels[((y + 2) * 4 + dy) * 60 + (x + 2) * 4 + dx] = 0;
                }
            }
        }
    }
    let image = tessipher::GrayImage::new(&pixels, 60, 60, 60).unwrap();
    start_counting();
    let matrix_result =
        tessipher::decode_micro_matrix(matrix, &mut core, &mut output, &mut segments);
    let image_result = tessipher::decode_micro_gray(
        image,
        &config,
        &mut detector,
        &mut core,
        &mut output,
        &mut segments,
    );
    let allocations = stop_counting();
    assert!(matrix_result.is_ok());
    assert!(image_result.is_ok());
    assert_eq!(allocations, 0);
}
