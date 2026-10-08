#![cfg(feature = "micro-qr-detect")]

use qr_ffi::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAllocator;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn fixture() -> (&'static [u8], usize) {
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    let output_len = u16::from_le_bytes([corpus[8], corpus[9]]) as usize;
    let matrix_len = u16::from_le_bytes([corpus[10], corpus[11]]) as usize;
    (&corpus[12 + output_len..12 + output_len + matrix_len], 11)
}

#[test]
fn micro_c_matrix_and_image_calls_do_not_allocate() {
    let (bits, side) = fixture();
    let mut config = QrConfig::default();
    assert_eq!(unsafe { qr_default_config(&mut config) }, 0);
    let mut memory = QrMicroMemory::default();
    assert_eq!(unsafe { qr_micro_memory(&config, &mut memory) }, 0);
    let mut matrix_scratch = vec![0; memory.matrix_core_scratch_bytes];
    let mut detector_scratch = vec![0; memory.detector_scratch_bytes];
    let mut image_scratch = vec![0; memory.image_core_scratch_bytes];
    let mut payload = [0; 35];
    let mut segments = [QrSegment::default(); 16];
    let mut result = QrMicroResult::default();
    let mut pixels = [255; 60 * 60];
    for y in 0..side {
        for x in 0..side {
            let bit = y * side + x;
            if bits[bit / 8] & (0x80 >> (bit % 8)) == 0 {
                continue;
            }
            for dy in 0..4 {
                for dx in 0..4 {
                    pixels[((y + 2) * 4 + dy) * 60 + (x + 2) * 4 + dx] = 0;
                }
            }
        }
    }

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let matrix_status = unsafe {
        qr_decode_micro_matrix(
            bits.as_ptr(),
            bits.len(),
            side,
            matrix_scratch.as_mut_ptr(),
            matrix_scratch.len(),
            payload.as_mut_ptr(),
            payload.len(),
            segments.as_mut_ptr(),
            segments.len(),
            &mut result,
        )
    };
    let image_status = unsafe {
        qr_decode_micro_gray(
            &config,
            pixels.as_ptr(),
            pixels.len(),
            60,
            60,
            60,
            detector_scratch.as_mut_ptr(),
            detector_scratch.len(),
            image_scratch.as_mut_ptr(),
            image_scratch.len(),
            payload.as_mut_ptr(),
            payload.len(),
            segments.as_mut_ptr(),
            segments.len(),
            &mut result,
        )
    };
    COUNTING.store(false, Ordering::Relaxed);

    assert_eq!(matrix_status, 0);
    assert_eq!(image_status, 0);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
}
