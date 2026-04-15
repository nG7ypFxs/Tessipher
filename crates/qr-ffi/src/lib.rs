//! Thin allocation-free C ABI. See the generated `tessipher.h` and `docs/ffi.md`.
#![cfg_attr(not(feature = "host"), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
use core::{
    mem::{align_of, size_of},
    ptr, slice,
};
use qr::{core as qcore, detect};
include!(concat!(env!("OUT_DIR"), "/dimensions.rs"));
macro_rules! abi_struct {($rust:ident => $c:literal {$($field:ident:$ty:ident),*})=>{#[derive(Clone,Copy,Debug,Default)]#[repr(C)]pub struct $rust{$(pub $field:$ty),*}};}
include!("../abi_schema.rs");
abi_schema!();
#[cfg(not(feature = "host"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
const _: () = {
    assert!(size_of::<QrSegment>() == size_of::<qcore::Segment>());
    assert!(align_of::<QrSegment>() == align_of::<qcore::Segment>());
    assert!(
        core::mem::offset_of!(QrSegment, offset) == core::mem::offset_of!(qcore::Segment, offset)
    );
    assert!(core::mem::offset_of!(QrSegment, len) == core::mem::offset_of!(qcore::Segment, len));
    assert!(core::mem::offset_of!(QrSegment, eci) == core::mem::offset_of!(qcore::Segment, eci));
};
#[derive(Clone, Copy)]
struct Range {
    start: usize,
    end: usize,
}
fn range<T>(p: *const T, n: usize) -> Result<Range, i32> {
    let bytes = n.checked_mul(size_of::<T>()).ok_or(1)?;
    let start = p as usize;
    if bytes > isize::MAX as usize || (bytes != 0 && (p.is_null() || start % align_of::<T>() != 0))
    {
        return Err(1);
    }
    let end = start.checked_add(bytes).ok_or(1)?;
    Ok(Range { start, end })
}
fn disjoint(ranges: &[Range]) -> Result<(), i32> {
    for (i, a) in ranges.iter().enumerate() {
        if a.start == a.end {
            continue;
        }
        for b in &ranges[i + 1..] {
            if b.start != b.end && a.start < b.end && b.start < a.end {
                return Err(2);
            }
        }
    }
    Ok(())
}
unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if n == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(p, n) }
    }
}
unsafe fn bytes_mut<'a>(p: *mut u8, n: usize) -> &'a mut [u8] {
    if n == 0 {
        &mut []
    } else {
        unsafe {
            ptr::write_bytes(p, 0, n);
            slice::from_raw_parts_mut(p, n)
        }
    }
}
unsafe fn segments_mut<'a>(p: *mut QrSegment, n: usize) -> &'a mut [qcore::Segment] {
    if n == 0 {
        return &mut [];
    }
    let ptr = p.cast::<qcore::Segment>();
    // C memory can be uninitialized. Establish valid Rust enum values before making a slice.
    for i in 0..n {
        unsafe {
            ptr.add(i).write(qcore::Segment::EMPTY);
        }
    }
    unsafe { slice::from_raw_parts_mut(ptr, n) }
}
fn config(c: QrConfig) -> detect::Config<MAX_WIDTH, MAX_HEIGHT> {
    detect::Config {
        max_finders: c.max_finders as usize,
        max_candidates: c.max_candidates as usize,
        max_retries: c.max_retries as usize,
        row_step: c.row_step as usize,
        local_radius: c.local_radius as usize,
    }
}
fn status(e: detect::Error) -> i32 {
    match e {
        detect::Error::InvalidImage => 10,
        detect::Error::ImageTooLarge => 11,
        detect::Error::InvalidConfig => 12,
        detect::Error::ScratchTooSmall => 13,
        detect::Error::FinderCapacity => 14,
        detect::Error::CandidateBudget => 15,
        detect::Error::RetryBudget => 16,
        detect::Error::NotFound => 17,
        detect::Error::Decode(e) => 100 + e as i32,
    }
}
fn result(d: qcore::Decoded) -> QrResult {
    let mut r = QrResult {
        version: d.version as u32,
        ec_level: d.ec_level as u32,
        mask: d.mask as u32,
        payload_len: d.payload_len,
        segment_count: d.segment_count,
        corrected_codewords: d.corrected_codewords as u32,
        format_distance: d.format_distance as u32,
        version_distance: d.version_distance as u32,
        ..QrResult::default()
    };
    match d.fnc1 {
        qcore::Fnc1::None => (),
        qcore::Fnc1::First => r.fnc1 = 1,
        qcore::Fnc1::Second(n) => {
            r.fnc1 = 2;
            r.fnc1_application_indicator = n as u32;
        }
    }
    if let Some(sa) = d.structured_append {
        r.sa_present = 1;
        r.sa_index = sa.index as u32;
        r.sa_total = sa.total as u32;
        r.sa_parity = sa.parity as u32;
    }
    r
}
/// Writes the default runtime budgets.
/// # Safety
/// `out` must point to a writable aligned `QrConfig` allocation.
#[no_mangle]
pub unsafe extern "C" fn qr_default_config(out: *mut QrConfig) -> i32 {
    if range(out, 1).is_err() {
        return 1;
    }
    let c = detect::Config::<MAX_WIDTH, MAX_HEIGHT>::default();
    unsafe {
        out.write(QrConfig {
            max_finders: c.max_finders as u32,
            max_candidates: c.max_candidates as u32,
            max_retries: c.max_retries as u32,
            row_step: c.row_step as u32,
            local_radius: c.local_radius as u32,
        });
    }
    0
}
/// Queries compiled dimensions, memory sizes and alignment.
/// # Safety
/// Both pointers must address valid, aligned, nonoverlapping allocations; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn qr_memory(c: *const QrConfig, out: *mut QrMemory) -> i32 {
    let run = || -> Result<(), i32> {
        disjoint(&[range(c, 1)?, range(out, 1)?])?;
        let cfg = config(unsafe { c.read() });
        let detector_scratch_bytes = cfg.scratch_bytes().map_err(status)?;
        let r = QrMemory {
            abi_version: 1,
            max_width: MAX_WIDTH as u32,
            max_height: MAX_HEIGHT as u32,
            core_scratch_bytes: qcore::SCRATCH_BYTES,
            detector_scratch_bytes,
            scratch_alignment: 1,
            max_payload_bytes: qcore::MAX_PAYLOAD_BYTES,
            max_segments: qcore::MAX_SEGMENTS,
            segment_size: size_of::<QrSegment>(),
            segment_alignment: align_of::<QrSegment>(),
            result_size: size_of::<QrResult>(),
            result_alignment: align_of::<QrResult>(),
        };
        unsafe {
            out.write(r);
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
/// Decodes a packed matrix.
/// # Safety
/// All spans must address allocations of the declared lengths; writable spans and result
/// must be exclusive, disjoint from every input, and correctly aligned. See docs/ffi.md.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_matrix(
    bits: *const u8,
    bits_len: usize,
    side: usize,
    scratch: *mut u8,
    scratch_len: usize,
    payload: *mut u8,
    payload_len: usize,
    segments: *mut QrSegment,
    segment_capacity: usize,
    out: *mut QrResult,
) -> i32 {
    let run = || -> Result<(), i32> {
        if segment_capacity > qcore::MAX_SEGMENTS {
            return Err(1);
        }
        disjoint(&[
            range(bits, bits_len)?,
            range(scratch, scratch_len)?,
            range(payload, payload_len)?,
            range(segments, segment_capacity)?,
            range(out, 1)?,
        ])?;
        let m = qcore::Matrix::new(unsafe { bytes(bits, bits_len) }, side)
            .map_err(|e| 100 + e as i32)?;
        let decoded = qcore::decode(
            m,
            unsafe { bytes_mut(scratch, scratch_len) },
            unsafe { bytes_mut(payload, payload_len) },
            unsafe { segments_mut(segments, segment_capacity) },
        )
        .map_err(|e| 100 + e as i32)?;
        unsafe {
            ptr::write(out, result(decoded));
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
/// Decodes one Gray8 ROI.
/// # Safety
/// All pointers must satisfy the allocation, length, alignment and exclusivity contract in
/// docs/ffi.md. This function checks numeric bounds and overlap, but cannot prove allocation validity.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_gray(
    c: *const QrConfig,
    pixels: *const u8,
    pixels_len: usize,
    width: usize,
    height: usize,
    stride: usize,
    detector_scratch: *mut u8,
    detector_scratch_len: usize,
    core_scratch: *mut u8,
    core_scratch_len: usize,
    payload: *mut u8,
    payload_len: usize,
    segments: *mut QrSegment,
    segment_capacity: usize,
    out: *mut QrResult,
) -> i32 {
    let run = || -> Result<(), i32> {
        if segment_capacity > qcore::MAX_SEGMENTS {
            return Err(1);
        }
        disjoint(&[
            range(c, 1)?,
            range(pixels, pixels_len)?,
            range(detector_scratch, detector_scratch_len)?,
            range(core_scratch, core_scratch_len)?,
            range(payload, payload_len)?,
            range(segments, segment_capacity)?,
            range(out, 1)?,
        ])?;
        let cfg = config(unsafe { c.read() });
        let image =
            detect::GrayImage::new(unsafe { bytes(pixels, pixels_len) }, width, height, stride)
                .map_err(status)?;
        let d = detect::decode(
            image,
            &cfg,
            unsafe { bytes_mut(detector_scratch, detector_scratch_len) },
            unsafe { bytes_mut(core_scratch, core_scratch_len) },
            unsafe { bytes_mut(payload, payload_len) },
            unsafe { segments_mut(segments, segment_capacity) },
        )
        .map_err(status)?;
        let mut r = result(d.decoded);
        r.inverted = d.inverted as u32;
        r.local_threshold = d.local_threshold as u32;
        r.candidates = d.stats.candidates;
        r.retries = d.stats.retries;
        [
            r.corner0_x,
            r.corner0_y,
            r.corner1_x,
            r.corner1_y,
            r.corner2_x,
            r.corner2_y,
            r.corner3_x,
            r.corner3_y,
        ] = [
            d.corners[0].x,
            d.corners[0].y,
            d.corners[1].x,
            d.corners[1].y,
            d.corners[2].x,
            d.corners[2].y,
            d.corners[3].x,
            d.corners[3].y,
        ];
        unsafe {
            out.write(r);
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
