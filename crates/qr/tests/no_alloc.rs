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
    ACTIVE.store(true, Ordering::SeqCst);
    let a = qr::decode_matrix(m, &mut cs, &mut out, &mut seg);
    let b = qr::decode_gray(img, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    let c = qr::decode_matrix(mirrored_matrix, &mut cs, &mut out, &mut seg);
    let d = qr::decode_gray(mirrored_image, &cfg, &mut ds, &mut cs, &mut out, &mut seg);
    ACTIVE.store(false, Ordering::SeqCst);
    assert!(a.is_ok() && b.is_ok() && c.is_ok() && d.is_ok());
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0);
}
