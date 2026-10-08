use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct Count;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;
#[test]
fn decoding_does_not_allocate() {
    let bits = include_bytes!("../../../fixtures/v1.bin");
    let m = qr::Matrix::new(bits, 21).unwrap();
    let mut cs = vec![0; qr::core::SCRATCH_BYTES];
    let cfg = qr::DefaultConfig::default();
    let mut ds = vec![0; cfg.scratch_bytes().unwrap()];
    let mut out = [0; 16];
    let mut seg = [qr::Segment::EMPTY; 8];
    let pgm = include_bytes!("../../../fixtures/v1.pgm");
    let offset = pgm
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(2)
        .unwrap()
        .0
        + 1;
    let img = qr::GrayImage::new(&pgm[offset..], 145, 145, 145).unwrap();
    let mut mirrored_bits = [0u8; 56];
    for y in 0..21 {
        for x in 0..21 {
            if m.get(y, x) == Some(true) {
                let i = y * 21 + x;
                mirrored_bits[i / 8] |= 128 >> (i % 8);
            }
        }
    }
    let mirrored_matrix = qr::Matrix::new(&mirrored_bits, 21).unwrap();
    let mut mirrored_pixels = pgm[offset..].to_vec();
    for row in mirrored_pixels.chunks_exact_mut(145) {
        row.reverse();
    }
    let mirrored_image = qr::GrayImage::new(&mirrored_pixels, 145, 145, 145).unwrap();
    let mut damaged_pixels = pgm[offset..].to_vec();
    for y in 0..21 {
        for x in 0..21 {
            if !qr::core::is_function(1, x, y) {
                let i = ((y + 4) * 5 + 2) * 145 + (x + 4) * 5 + 2;
                damaged_pixels[i] = 255 - damaged_pixels[i];
            }
        }
    }
    let damaged_image = qr::GrayImage::new(&damaged_pixels, 145, 145, 145).unwrap();
    let mut stable_pixels = vec![255; 400 * 300];
    let mut moved_pixels = vec![255; 400 * 300];
    for y in 0..145 {
        stable_pixels[(y + 40) * 400 + 40..(y + 40) * 400 + 185]
            .copy_from_slice(&pgm[offset + y * 145..offset + (y + 1) * 145]);
        moved_pixels[(y + 140) * 400 + 220..(y + 140) * 400 + 365]
            .copy_from_slice(&pgm[offset + y * 145..offset + (y + 1) * 145]);
    }
    let stable_image = qr::GrayImage::new(&stable_pixels, 400, 300, 400).unwrap();
    let moved_image = qr::GrayImage::new(&moved_pixels, 400, 300, 400).unwrap();
    let blank_pixels = vec![255; 400 * 300];
    let blank_image = qr::GrayImage::new(&blank_pixels, 400, 300, 400).unwrap();
    let mut tracker = qr::Tracker::new();
    let mut stats = qr::Stats::default();
    ACTIVE.store(true, Ordering::SeqCst);
    let a = qr::decode_matrix(m, &mut cs, &mut out, &mut seg);
    let b = qr::decode_gray(img, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    let c = qr::decode_matrix(mirrored_matrix, &mut cs, &mut out, &mut seg);
    let d = qr::decode_gray(mirrored_image, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    let e = qr::decode_gray_with_sampling(
        img,
        &cfg,
        qr::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let f = tracker.decode_with_stats(
        img,
        &cfg,
        qr::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let g = tracker.decode_with_stats(
        img,
        &cfg,
        qr::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let h = qr::decode_gray_with_sampling(
        damaged_image,
        &cfg,
        qr::SamplingStrategy::CenterThenMajority5,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let i = tracker.decode_with_stats(
        stable_image,
        &cfg,
        qr::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    tracker = qr::Tracker::from_state(tracker.state()).unwrap();
    let j = tracker.decode_with_stats(
        stable_image,
        &cfg,
        qr::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let k = tracker.decode_with_stats(
        moved_image,
        &cfg,
        qr::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    let l = tracker.decode_with_stats(
        blank_image,
        &cfg,
        qr::SamplingStrategy::Center,
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
        qr::SamplingStrategy::Center,
        &mut ds,
        &mut cs,
        &mut out,
        &mut seg,
        &mut stats,
    );
    ACTIVE.store(false, Ordering::SeqCst);
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
    assert_eq!(l, Err(qr::detect::Error::NotFound));
    assert_eq!(n, Err(qr::detect::Error::NotFound));
    assert!(matches!(candidate, qr::TrackerState::Candidate { .. }));
    assert_eq!(tracker.state(), qr::TrackerState::Searching);
    assert_eq!(h.unwrap().stats.retries, 2);
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0);
}
