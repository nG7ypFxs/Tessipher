//! Allocation-free C ABI for QR Model 2 and opt-in Micro QR decoding.
//!
//! Call [`qr_default_config`] and [`qr_memory`] before allocating caller-owned
//! storage. [`qr_decode_matrix`] and [`qr_decode_gray`] borrow input and initialize
//! writable scratch, payload, and segment storage before constructing Rust slices.
//! No allocator is required. With the default `host` feature this crate links std;
//! firmware builds use `--no-default-features --release` and a looping panic handler.
//!
//! All functions return 0 on success, 1 for invalid numeric pointer/length/alignment
//! arguments, and 2 for overlapping nonempty ranges. Detector errors are 10..=18:
//! invalid image, oversized image, invalid config, short detector scratch, finder
//! capacity, candidate budget, retry budget, not found, and pixel-read budget. Core errors are
//! `100 + tessipher::core::Error` (101..=110). Result structs are written only on success.
//! Buffer contents are unspecified on error; only returned prefixes are valid on
//! success. Numeric checks cannot establish pointer allocation validity.
//!
//! The generated C header `include/tessipher.h` and repository `docs/ffi.md`
//! describe the same ABI and embedded integration contract.
#![cfg_attr(not(feature = "host"), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
use core::{
    mem::{align_of, size_of},
    ptr, slice,
};
use tessipher::{core as qcore, detect};
include!(concat!(env!("OUT_DIR"), "/dimensions.rs"));
macro_rules! abi_struct {
    ($(#[doc = $doc:literal])* $rust:ident => $c:literal {
        $($(#[doc = $field_doc:literal])* $field:ident:$ty:ident),*
    }) => {
        $(#[doc = $doc])*
        #[derive(Clone, Copy, Debug, Default)]
        #[repr(C)]
        pub struct $rust {
            $($(#[doc = $field_doc])* pub $field: $ty),*
        }
    };
}
include!("../abi_schema.rs");
abi_schema!();
/// Sentinel stored in [`QrMicroResult::ec_level`] for M1 detection-only parity.
#[cfg(feature = "micro-qr")]
pub const QR_MICRO_EC_NONE: u32 = u32::MAX;
#[cfg(feature = "micro-qr")]
const MICRO_MATRIX_CORE_SCRATCH_BYTES: usize = 184;
#[cfg(feature = "micro-qr")]
const MICRO_IMAGE_CORE_SCRATCH_BYTES: usize = MICRO_MATRIX_CORE_SCRATCH_BYTES;
#[cfg(feature = "micro-qr")]
const MICRO_MAX_PAYLOAD_BYTES: usize = 35;
#[cfg(feature = "micro-qr")]
const MICRO_MAX_SEGMENTS: usize = 16;
#[cfg(feature = "micro-qr")]
const _: () = {
    assert!(qcore::MICRO_SCRATCH_BYTES == MICRO_MATRIX_CORE_SCRATCH_BYTES);
    assert!(qcore::MICRO_MAX_PAYLOAD_BYTES == MICRO_MAX_PAYLOAD_BYTES);
};
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
        max_pixel_reads: detect::DEFAULT_MAX_PIXEL_READS,
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
        detect::Error::PixelBudget => 18,
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
#[cfg(feature = "micro-qr")]
fn micro_result(d: qcore::MicroDecoded) -> QrMicroResult {
    QrMicroResult {
        version: d.version as u32,
        ec_level: d.ec_level.map_or(QR_MICRO_EC_NONE, |level| level as u32),
        mask: d.mask as u32,
        payload_len: d.payload_len,
        segment_count: d.segment_count,
        corrected_codewords: d.corrected_codewords as u32,
        format_distance: d.format_distance as u32,
        rotation: d.rotation as u32,
        mirrored: d.mirrored as u32,
        ..QrMicroResult::default()
    }
}
/// Writes default runtime budgets: 32 finders, 4096 candidates, 256 sampling
/// attempts, row step 1, and local radius 24.
///
/// Returns 0 on success or 1 for a null, misaligned, or numerically invalid pointer.
/// Does not allocate; `out` need not be initialized.
/// # Safety
/// `out` must point to a live, writable, properly aligned allocation for one
/// [`QrConfig`], with exclusive access for the duration of the call.
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
/// Queries compiled dimensions, scratch requirements, and target struct layouts.
///
/// Returns 0 on success, 1 for invalid pointer arithmetic/alignment, 2 for overlap,
/// or 12 for invalid configuration. `out` is written only on success.
/// # Safety
/// `c` must point to one initialized, readable [`QrConfig`] that remains unchanged.
/// `out` must point to writable storage for one [`QrMemory`], with exclusive access.
/// Both allocations must remain live, aligned, and mutually disjoint throughout
/// the call. `out` need not be initialized.
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
/// Queries byte-aligned scratch required by Model 2 matrix decoding for version 1..=40.
/// Invalid versions return QR_VERSION (104). `out` is written only on success. Image decoding uses qr_memory instead.
///
/// # Safety
/// `out` must point to an aligned, writable `size_t`.
#[no_mangle]
pub unsafe extern "C" fn qr_matrix_scratch_bytes(version: u32, out: *mut usize) -> i32 {
    let run = || -> Result<(), i32> {
        range(out, 1)?;
        let version = u8::try_from(version).map_err(|_| qcore::Error::Version as i32 + 100)?;
        let needed = qcore::scratch_bytes(version).map_err(|e| e as i32 + 100)?;
        unsafe { out.write(needed) };
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// Queries the linked Micro QR limits, scratch requirements, and
/// additive C type layouts without changing [`QrMemory`] or ABI version 1.
///
/// The query and matrix decoder are exported with Cargo feature `micro-qr`.
/// Returns 0, 1 for invalid pointer arithmetic/alignment, 2 for overlap, or 12
/// for invalid configuration. `out` is written only on success.
/// # Safety
/// `c` must point to one initialized, readable [`QrConfig`] and `out` to one
/// live, aligned, exclusively writable [`QrMicroMemory`]. Their allocations
/// must remain live and disjoint for the duration of the call.
#[no_mangle]
#[cfg(feature = "micro-qr")]
pub unsafe extern "C" fn qr_micro_memory(c: *const QrConfig, out: *mut QrMicroMemory) -> i32 {
    let run = || -> Result<(), i32> {
        disjoint(&[range(c, 1)?, range(out, 1)?])?;
        let detector_scratch_bytes = config(unsafe { c.read() })
            .scratch_bytes()
            .map_err(status)?;
        unsafe {
            out.write(QrMicroMemory {
                abi_version: 1,
                max_width: MAX_WIDTH as u32,
                max_height: MAX_HEIGHT as u32,
                matrix_core_scratch_bytes: MICRO_MATRIX_CORE_SCRATCH_BYTES,
                image_core_scratch_bytes: if cfg!(feature = "micro-qr-detect") {
                    MICRO_IMAGE_CORE_SCRATCH_BYTES
                } else {
                    0
                },
                detector_scratch_bytes: if cfg!(feature = "micro-qr-detect") {
                    detector_scratch_bytes
                } else {
                    0
                },
                scratch_alignment: 1,
                max_payload_bytes: MICRO_MAX_PAYLOAD_BYTES,
                max_segments: MICRO_MAX_SEGMENTS,
                segment_size: size_of::<QrSegment>(),
                segment_alignment: align_of::<QrSegment>(),
                result_size: size_of::<QrMicroResult>(),
                result_alignment: align_of::<QrMicroResult>(),
                stats_size: size_of::<QrStats>(),
                stats_alignment: align_of::<QrStats>(),
            });
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// Decodes a packed row-major, MSB-first M1 through M4 matrix.
///
/// `side` is 11, 13, 15, or 17 and the quiet zone is excluded. Supply the
/// `matrix_core_scratch_bytes` reported by [`qr_micro_memory`]. M1 reports
/// [`QR_MICRO_EC_NONE`]. This symbol is exported with Cargo feature `micro-qr`.
/// # Safety
/// The allocation, lifetime, alignment, exclusivity, initialization, and
/// disjointness requirements of [`qr_decode_matrix`] apply to the corresponding
/// spans and to one writable [`QrMicroResult`] at `out`.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
#[cfg(feature = "micro-qr")]
pub unsafe extern "C" fn qr_decode_micro_matrix(
    bits: *const u8,
    bits_len: usize,
    side: usize,
    scratch: *mut u8,
    scratch_len: usize,
    payload: *mut u8,
    payload_len: usize,
    segments: *mut QrSegment,
    segment_capacity: usize,
    out: *mut QrMicroResult,
) -> i32 {
    let run = || -> Result<(), i32> {
        if segment_capacity > MICRO_MAX_SEGMENTS {
            return Err(1);
        }
        disjoint(&[
            range(bits, bits_len)?,
            range(scratch, scratch_len)?,
            range(payload, payload_len)?,
            range(segments, segment_capacity)?,
            range(out, 1)?,
        ])?;
        let matrix = qcore::MicroMatrix::new(unsafe { bytes(bits, bits_len) }, side)
            .map_err(|e| 100 + e as i32)?;
        let decoded = qcore::decode_micro(
            matrix,
            unsafe { bytes_mut(scratch, scratch_len) },
            unsafe { bytes_mut(payload, payload_len) },
            unsafe { segments_mut(segments, segment_capacity) },
        )
        .map_err(|e| 100 + e as i32)?;
        unsafe {
            out.write(micro_result(decoded));
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
/// Decodes a packed row-major, MSB-first matrix without row padding.
///
/// `side` must be `17 + 4 * version` for version 1..=40. Supply at least
/// [`qr_matrix_scratch_bytes`] scratch bytes for the version. Segment capacity may not exceed
/// [`tessipher::core::MAX_SEGMENTS`]. Both output capacities may be smaller than the
/// maximum; insufficient space returns a core capacity error.
/// Normal orientation is tried first, then its transpose after content errors.
///
/// Returns 0 on success, 1 for invalid numeric arguments, 2 for overlap, or
/// 101..=110 for core errors. Read only the prefixes specified in `out` on success.
/// `out` is untouched on error; other writable buffer contents are unspecified.
/// Detection-only result fields are zero.
/// # Safety
/// Every nonempty span must lie in one live allocation of its declared length and
/// alignment, with the input initialized and unchanged during the call. Scratch,
/// payload, segments, and `out` require exclusive writable access and need not be
/// initialized. All nonempty ranges, including inputs, must be mutually disjoint.
/// `out` is mandatory and must hold one aligned [`QrResult`]. Null is allowed only
/// for zero-length byte/segment spans. No concurrent thread, DMA, or interrupt may
/// access writable ranges or modify input. Numeric validation cannot prove these
/// allocation, lifetime, or access requirements.
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
#[cfg(feature = "micro-qr-detect")]
fn micro_detection_result(d: detect::MicroDetection) -> QrMicroResult {
    let mut r = micro_result(d.decoded);
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
    r
}

/// Decodes the first Micro QR symbol in a Gray8 ROI using the finite default
/// pixel-read budget and center-then-majority-five sampling.
///
/// This symbol is exported with Cargo feature `micro-qr-detect`. Query
/// [`qr_micro_memory`] for the detector and image-core scratch sizes.
/// # Safety
/// All allocation, lifetime, initialization, alignment, exclusive-access, and
/// disjointness requirements of [`qr_decode_gray`] apply to corresponding spans
/// and to one writable [`QrMicroResult`] at `out`.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_micro_gray(
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
    out: *mut QrMicroResult,
) -> i32 {
    unsafe {
        qr_decode_micro_gray_with_budget(
            c,
            detect::DEFAULT_MAX_PIXEL_READS,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
        )
    }
}

/// Like [`qr_decode_micro_gray`], with an explicit shared input-pixel-read limit.
/// Zero returns invalid-config status 12; exhaustion returns pixel-budget status 18.
/// This symbol is exported with Cargo feature `micro-qr-detect`.
/// # Safety
/// All requirements of [`qr_decode_micro_gray`] apply without change.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_micro_gray_with_budget(
    c: *const QrConfig,
    max_pixel_reads: usize,
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
    out: *mut QrMicroResult,
) -> i32 {
    unsafe {
        qr_decode_micro_gray_with_sampling(
            c,
            max_pixel_reads,
            QR_DEFAULT_SAMPLING_STRATEGY,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            ptr::null_mut(),
        )
    }
}

#[cfg(feature = "micro-qr-detect")]
struct MicroGrayCall {
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    out: *mut QrMicroResult,
    stats: *mut QrStats,
}

#[cfg(feature = "micro-qr-detect")]
unsafe fn micro_gray_call(a: MicroGrayCall) -> i32 {
    let run = || -> Result<(), i32> {
        if a.segment_capacity > MICRO_MAX_SEGMENTS {
            return Err(1);
        }
        disjoint(&[
            range(a.c, 1)?,
            range(a.pixels, a.pixels_len)?,
            range(a.detector_scratch, a.detector_scratch_len)?,
            range(a.core_scratch, a.core_scratch_len)?,
            range(a.payload, a.payload_len)?,
            range(a.segments, a.segment_capacity)?,
            range(a.out, 1)?,
            range(a.stats, usize::from(!a.stats.is_null()))?,
        ])?;
        let strategy = sampling(a.strategy)?;
        let mut cfg = config(unsafe { a.c.read() });
        cfg.max_pixel_reads = a.max_pixel_reads;
        cfg.scratch_bytes().map_err(status)?;
        let image = detect::GrayImage::new(
            unsafe { bytes(a.pixels, a.pixels_len) },
            a.width,
            a.height,
            a.stride,
        )
        .map_err(status)?;
        let mut stats = detect::Stats::default();
        // All ranges, enum values, configuration, and image dimensions were checked first.
        let decoded = detect::decode_micro_with_sampling(
            image,
            &cfg,
            strategy,
            unsafe { bytes_mut(a.detector_scratch, a.detector_scratch_len) },
            unsafe { bytes_mut(a.core_scratch, a.core_scratch_len) },
            unsafe { bytes_mut(a.payload, a.payload_len) },
            unsafe { segments_mut(a.segments, a.segment_capacity) },
            &mut stats,
        );
        if !a.stats.is_null() {
            unsafe {
                a.stats.write(work(stats));
            }
        }
        let found = decoded.map_err(status)?;
        unsafe {
            a.out.write(micro_detection_result(found));
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// Decodes a Micro QR Gray8 ROI with explicit pixel budget and sampling policy.
///
/// Strategies are [`QR_SAMPLING_CENTER`] and
/// [`QR_SAMPLING_CENTER_THEN_MAJORITY5`]. A null `stats` is allowed. Nonnull
/// stats are written after detector invocation on success or failure; pre-decode
/// rejection leaves stats untouched. Result storage is success-only. This symbol
/// is exported with Cargo feature `micro-qr-detect`.
/// # Safety
/// All requirements of [`qr_decode_micro_gray`] apply. Nonnull `stats` must name
/// one live, aligned, exclusively writable [`QrStats`] disjoint from every span.
#[cfg(feature = "micro-qr-detect")]
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_micro_gray_with_sampling(
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    out: *mut QrMicroResult,
    stats: *mut QrStats,
) -> i32 {
    unsafe {
        micro_gray_call(MicroGrayCall {
            c,
            max_pixel_reads,
            strategy,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            stats,
        })
    }
}
/// Decodes the first symbol found in a borrowed Gray8 ROI.
/// Uses center-then-majority-five sampling, with no hidden tracking state.
/// Explicit center selection is available through [`qr_decode_gray_with_sampling`].
/// Uses the finite default pixel-read limit [`detect::DEFAULT_MAX_PIXEL_READS`].
///
/// Pixels start at the ROI's top-left. Width and height must be nonzero,
/// stride at least width, and storage at least `(height - 1) * stride + width`.
/// The last row needs no trailing padding. Compiled limits are [`MAX_WIDTH`] and
/// [`MAX_HEIGHT`]; query [`qr_memory`] for scratch sizes and alignment.
/// Segment capacity may not exceed [`tessipher::core::MAX_SEGMENTS`].
///
/// Returns 0 on success, 1 for invalid numeric arguments, 2 for overlap,
/// 10..=18 for detector errors, or 101..=110 for core errors. Read only the returned
/// payload/segment prefixes on success. `out` is untouched on error; other writable
/// buffer contents are unspecified. Corners retain sampling order even if core
/// transpose decoding succeeds; there is no mirrored flag.
/// # Safety
/// Each nonempty span must lie in one live allocation of its declared length and
/// alignment. `c` must hold an initialized [`QrConfig`]; it and all input pixels
/// must remain unchanged during the call. Both scratch buffers, payload, segments,
/// and `out` require exclusive writable access and need not be initialized.
/// All nonempty ranges, including inputs, must be mutually disjoint. `c` and `out`
/// are mandatory; `out` must hold one aligned [`QrResult`]. Null is allowed only
/// for zero-length byte/segment spans. No concurrent thread, DMA, or interrupt may
/// access writable ranges or modify input. Numeric validation cannot prove these
/// allocation, lifetime, or access requirements.
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
    unsafe {
        qr_decode_gray_with_budget(
            c,
            detect::DEFAULT_MAX_PIXEL_READS,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
        )
    }
}
/// Like [`qr_decode_gray`], with an explicit shared limit on actual input pixel reads.
///
/// `max_pixel_reads` accepts 1..=usize::MAX. Zero returns 12; exhaustion returns 18
/// before another pixel read. Includes initial min/max, all passes and repeated
/// reads; local threshold decisions cost 26 reads. The default entry point uses
/// [`detect::DEFAULT_MAX_PIXEL_READS`]. This limit does not bound total CPU cycles
/// or initialization of caller-provided buffers. The ABI-1 config/result/memory
/// layouts remain unchanged. `out` is written only on success.
/// # Safety
/// All allocation, lifetime, alignment, disjointness and exclusive-access
/// requirements of [`qr_decode_gray`] apply without change.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_gray_with_budget(
    c: *const QrConfig,
    max_pixel_reads: usize,
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
    unsafe {
        qr_decode_gray_with_sampling(
            c,
            max_pixel_reads,
            QR_DEFAULT_SAMPLING_STRATEGY,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            ptr::null_mut(),
        )
    }
}

/// Legacy center sampling: no extra attempt after a failed matrix.
pub const QR_SAMPLING_CENTER: u32 = 0;
/// Center sampling with a five-vote fallback after core content failure.
pub const QR_SAMPLING_CENTER_THEN_MAJORITY5: u32 = 1;
/// Sampling policy used by the existing stateless Gray8 C entry points.
pub const QR_DEFAULT_SAMPLING_STRATEGY: u32 = QR_SAMPLING_CENTER_THEN_MAJORITY5;

fn sampling(n: u32) -> Result<detect::SamplingStrategy, i32> {
    match n {
        QR_SAMPLING_CENTER => Ok(detect::SamplingStrategy::Center),
        QR_SAMPLING_CENTER_THEN_MAJORITY5 => Ok(detect::SamplingStrategy::CenterThenMajority5),
        _ => Err(12),
    }
}
fn restore(t: QrTracker) -> Result<detect::Tracker, i32> {
    if t.image_width > MAX_WIDTH || t.image_height > MAX_HEIGHT {
        return Err(1);
    }
    let hint = detect::TrackHint {
        roi: detect::Rect {
            x: t.roi_x,
            y: t.roi_y,
            width: t.roi_width,
            height: t.roi_height,
        },
        image_width: t.image_width,
        image_height: t.image_height,
    };
    let state = match (t.tag, t.misses) {
        (0, 0)
            if t.roi_x == 0
                && t.roi_y == 0
                && t.roi_width == 0
                && t.roi_height == 0
                && t.image_width == 0
                && t.image_height == 0 =>
        {
            detect::TrackerState::Searching
        }
        (1, 1) => detect::TrackerState::Candidate { hint, misses: 1 },
        (2, 0) => detect::TrackerState::Tracking { hint },
        _ => return Err(1),
    };
    detect::Tracker::from_state(state).map_err(|_| 1)
}
fn save(state: detect::TrackerState) -> QrTracker {
    let (tag, misses, hint) = match state {
        detect::TrackerState::Searching => return QrTracker::default(),
        detect::TrackerState::Candidate { hint, misses } => (1, misses as u32, hint),
        detect::TrackerState::Tracking { hint } => (2, 0, hint),
    };
    QrTracker {
        tag,
        misses,
        roi_x: hint.roi.x,
        roi_y: hint.roi.y,
        roi_width: hint.roi.width,
        roi_height: hint.roi.height,
        image_width: hint.image_width,
        image_height: hint.image_height,
    }
}
fn work(s: detect::Stats) -> QrStats {
    QrStats {
        pixel_reads: s.pixel_reads,
        rows: s.rows,
        finder_observations: s.finder_observations,
        candidates: s.candidates,
        retries: s.retries,
        passes: s.passes,
    }
}
fn detection_result(d: detect::Detection) -> QrResult {
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
    r
}
/// Queries target tracker/stats sizes without changing the ABI-1 memory query.
/// `out` need not be initialized. Returns 0 or invalid-argument status 1.
/// # Safety
/// `out` must name one live, aligned, exclusively writable [`QrTrackerMemory`].
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_memory(out: *mut QrTrackerMemory) -> i32 {
    if range(out, 1).is_err() {
        return 1;
    }
    unsafe {
        out.write(QrTrackerMemory {
            tracker_size: size_of::<QrTracker>(),
            tracker_alignment: align_of::<QrTracker>(),
            stats_size: size_of::<QrStats>(),
            stats_alignment: align_of::<QrStats>(),
        });
    }
    0
}
/// Writes canonical Searching without reading the previous storage.
/// `out` may be uninitialized. Returns 0 or invalid-argument status 1.
/// # Safety
/// `out` must name one live, aligned, exclusively writable [`QrTracker`].
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_init(out: *mut QrTracker) -> i32 {
    if range(out, 1).is_err() {
        return 1;
    }
    unsafe {
        out.write(QrTracker::default());
    }
    0
}
/// Discards all saved state; has the same write-only contract as [`qr_tracker_init`].
/// # Safety
/// `out` must name one live, aligned, exclusively writable [`QrTracker`].
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_reset(out: *mut QrTracker) -> i32 {
    unsafe { qr_tracker_init(out) }
}
/// Queries optional local cache bytes (alignment one), preserving ABI-1 memory layouts.
/// Zero or oversized widths and invalid configurations return 12. Output is
/// written only on success.
/// # Safety
/// `c` is readable and initialized; `out` names one exclusive writable usize.
/// Both objects must be aligned, live, and disjoint.
#[no_mangle]
pub unsafe extern "C" fn qr_local_cache_memory(
    c: *const QrConfig,
    width: usize,
    out: *mut usize,
) -> i32 {
    let run = || -> Result<(), i32> {
        disjoint(&[range(c, 1)?, range(out, 1)?])?;
        let needed = config(unsafe { c.read() })
            .local_cache_bytes(width)
            .map_err(status)?;
        unsafe {
            out.write(needed);
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
#[derive(Clone, Copy)]
struct GrayCall {
    cache: Option<(*mut u8, usize)>,
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
    tracker: Option<*mut QrTracker>,
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
    stats: *mut QrStats,
}
unsafe fn gray_call(a: GrayCall) -> i32 {
    let run = || -> Result<(), i32> {
        if a.segment_capacity > qcore::MAX_SEGMENTS {
            return Err(1);
        }
        disjoint(&[
            match a.cache {
                Some((p, n)) => range(p, n)?,
                None => Range { start: 0, end: 0 },
            },
            range(a.c, 1)?,
            range(a.pixels, a.pixels_len)?,
            range(a.detector_scratch, a.detector_scratch_len)?,
            range(a.core_scratch, a.core_scratch_len)?,
            range(a.payload, a.payload_len)?,
            range(a.segments, a.segment_capacity)?,
            range(a.out, 1)?,
            range(a.stats, usize::from(!a.stats.is_null()))?,
            match a.tracker {
                Some(p) => range(p, 1)?,
                None => Range { start: 0, end: 0 },
            },
        ])?;
        let strategy = sampling(a.strategy)?;
        let mut tracker = match a.tracker {
            Some(p) => Some(restore(unsafe { p.read() })?),
            None => None,
        };
        let mut cfg = config(unsafe { a.c.read() });
        cfg.max_pixel_reads = a.max_pixel_reads;
        cfg.scratch_bytes().map_err(status)?;
        let image = detect::GrayImage::new(
            unsafe { bytes(a.pixels, a.pixels_len) },
            a.width,
            a.height,
            a.stride,
        )
        .map_err(status)?;
        let cache_needed = match a.cache {
            Some((_, n)) => {
                let needed = cfg.local_cache_bytes(a.width).map_err(status)?;
                if n < needed {
                    return Err(13);
                }
                needed
            }
            None => 0,
        };
        let previous = tracker.as_ref().map(detect::Tracker::state);
        let mut stats = detect::Stats::default();
        // All numeric ranges, state fields and overlaps were checked before initialization.
        let ds = unsafe { bytes_mut(a.detector_scratch, a.detector_scratch_len) };
        let cs = unsafe { bytes_mut(a.core_scratch, a.core_scratch_len) };
        let output = unsafe { bytes_mut(a.payload, a.payload_len) };
        let segments = unsafe { segments_mut(a.segments, a.segment_capacity) };
        // Initialize only the required prefix, even when C supplied excess capacity.
        let cache = a.cache.map(|(p, _)| unsafe { bytes_mut(p, cache_needed) });
        let decoded = match (tracker.as_mut(), cache) {
            (Some(t), Some(cache)) => t.decode_with_stats_cached(
                image, &cfg, strategy, ds, cs, output, segments, &mut stats, cache,
            ),
            (None, Some(cache)) => detect::decode_with_sampling_cached(
                image, &cfg, strategy, ds, cs, output, segments, &mut stats, cache,
            ),
            (Some(t), None) => {
                t.decode_with_stats(image, &cfg, strategy, ds, cs, output, segments, &mut stats)
            }
            (None, None) => detect::decode_with_sampling(
                image, &cfg, strategy, ds, cs, output, segments, &mut stats,
            ),
        };
        if !a.stats.is_null() {
            unsafe {
                a.stats.write(work(stats));
            }
        }
        if let (Some(p), Some(t)) = (a.tracker, tracker) {
            // Rust preserves its previous state on every terminal error. Do not write
            // C storage on that path, including after a changed input dimension.
            if decoded.is_ok() || Some(t.state()) != previous {
                unsafe {
                    p.write(save(t.state()));
                }
            }
        }
        let d = decoded.map_err(status)?;
        unsafe {
            a.out.write(detection_result(d));
        }
        Ok(())
    };
    match run() {
        Ok(()) => 0,
        Err(e) => e,
    }
}
/// Decodes Gray8 with explicit sampling, pixel budget, and optional failure counters.
/// Unknown strategies return 12; 0 selects center, 1 center-then-majority-five.
/// A null `stats` is allowed; nonnull storage is written after decoder invocation
/// on success or failure, but is untouched on earlier argument rejection.
/// # Safety
/// All [`qr_decode_gray`] pointer and exclusive-access requirements apply.
/// A nonnull `stats` must additionally name one aligned, exclusively writable
/// [`QrStats`] disjoint from every other range; it need not be initialized.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_gray_with_sampling(
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    stats: *mut QrStats,
) -> i32 {
    unsafe {
        gray_call(GrayCall {
            cache: None,
            c,
            max_pixel_reads,
            strategy,
            tracker: None,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            stats,
        })
    }
}

/// Optional internal-RAM five-row cache; all supplied spans must be disjoint.
/// Excess cache capacity is untouched. Input reads include fills and exclude hits.
/// # Safety
/// All pointer contracts of the corresponding uncached function apply. Cache
/// must name `cache_len` exclusively writable bytes, disjoint from every span.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_gray_with_sampling_cached(
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    stats: *mut QrStats,
    cache: *mut u8,
    cache_len: usize,
) -> i32 {
    unsafe {
        gray_call(GrayCall {
            cache: Some((cache, cache_len)),
            c,
            max_pixel_reads,
            strategy,
            tracker: None,
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            stats,
        })
    }
}
/// Decodes one tracked frame with explicit sampling, pixel budget, and optional stats.
///
/// Tracker integers are validated against saved dimensions before use; invalid
/// tags/fields return 1. Saved dimensions must fit this library's compiled limits.
/// Current input resize is valid and forces full search.
/// Only success or complete recognition failure updates state. Capacity, budget,
/// validation, or pointer errors leave state and result untouched. Result corners
/// refer to the complete supplied image. The tracker retains no borrowed storage.
/// # Safety
/// All [`qr_decode_gray_with_sampling`] requirements apply. `tracker` must name
/// one initialized, aligned, exclusively writable [`QrTracker`], disjoint from
/// every other range. Initialize with [`qr_tracker_init`] or zero-initialize all
/// fields before first use. Reading uninitialized tracker storage is invalid;
/// integer validation cannot prove initialization, allocation or exclusivity.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_decode_gray(
    tracker: *mut QrTracker,
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    stats: *mut QrStats,
) -> i32 {
    unsafe {
        gray_call(GrayCall {
            cache: None,
            c,
            max_pixel_reads,
            strategy,
            tracker: Some(tracker),
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            stats,
        })
    }
}

/// Optional internal-RAM five-row cache; all supplied spans must be disjoint.
/// Excess cache capacity is untouched. Input reads include fills and exclude hits.
/// # Safety
/// All pointer contracts of the corresponding uncached function apply. Cache
/// must name `cache_len` exclusively writable bytes, disjoint from every span.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_decode_gray_cached(
    tracker: *mut QrTracker,
    c: *const QrConfig,
    max_pixel_reads: usize,
    strategy: u32,
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
    stats: *mut QrStats,
    cache: *mut u8,
    cache_len: usize,
) -> i32 {
    unsafe {
        gray_call(GrayCall {
            cache: Some((cache, cache_len)),
            c,
            max_pixel_reads,
            strategy,
            tracker: Some(tracker),
            pixels,
            pixels_len,
            width,
            height,
            stride,
            detector_scratch,
            detector_scratch_len,
            core_scratch,
            core_scratch_len,
            payload,
            payload_len,
            segments,
            segment_capacity,
            out,
            stats,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
