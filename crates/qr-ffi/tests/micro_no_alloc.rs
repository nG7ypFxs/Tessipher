#![cfg(feature = "micro-qr-detect")]

use qr_ffi::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;
thread_local! {
    // Constant initialization avoids allocations in the allocator's TLS access.
    // Count only this thread so the test harness cannot affect the measurement.
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count_allocation() {
    // TLS may be unavailable during thread teardown; allocators must not panic.
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n.saturating_add(1)));
        }
    });
}

fn start_counting() {
    ALLOCATIONS.with(|count| {
        assert!(count.get().is_none());
        count.set(Some(0));
    });
}

fn stop_counting() -> usize {
    ALLOCATIONS.with(|count| {
        count
            .replace(None)
            .expect("allocation measurement is active")
    })
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
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
fn counting_ignores_other_threads() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let ready = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !ready.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            let layout = Layout::from_size_align(16, 8).unwrap();
            unsafe {
                let ptr = std::hint::black_box(ALLOCATOR.alloc(layout));
                assert!(!ptr.is_null());
                ALLOCATOR.dealloc(ptr, layout);
            }
            done.store(true, Ordering::Release);
        });
        // Parking may allocate on the measured thread; spin for synchronization.
        start_counting();
        ready.store(true, Ordering::Release);
        while !done.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        assert_eq!(stop_counting(), 0);
    });
}

#[test]
fn counting_detects_allocations_and_resets() {
    let layout = Layout::from_size_align(16, 8).unwrap();
    start_counting();
    // Direct calls keep the allocation operations observable in release builds.
    unsafe {
        let ptr = std::hint::black_box(ALLOCATOR.alloc(layout));
        assert!(!ptr.is_null());
        let resized = std::hint::black_box(ALLOCATOR.realloc(ptr, layout, 32));
        assert!(!resized.is_null());
        ALLOCATOR.dealloc(resized, Layout::from_size_align(32, 8).unwrap());
        let zeroed = std::hint::black_box(ALLOCATOR.alloc_zeroed(layout));
        assert!(!zeroed.is_null());
        ALLOCATOR.dealloc(zeroed, layout);
    }
    assert_eq!(stop_counting(), 3);

    start_counting();
    assert_eq!(stop_counting(), 0);
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

    start_counting();
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
    let allocations = stop_counting();

    assert_eq!(matrix_status, 0);
    assert_eq!(image_status, 0);
    assert_eq!(allocations, 0);
}
