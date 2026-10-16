//! Allocation-free C ABI for QR Model 2 and opt-in Micro QR decoding.
//!
//! Call [`qr_default_config`] and [`qr_memory`] before allocating caller-owned
//! storage; [`qr_detector_scratch_bytes`] recommends optional Fast scratch.
//! [`qr_decode_matrix`] and [`qr_decode_gray`] borrow input and initialize
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
    ptr,
};
use tessipher::{core as qcore, detect};
mod gray;
mod input;

use gray::detection_mode;
pub use gray::{
    qr_decode_gray, qr_decode_gray_with_budget, qr_decode_gray_with_mode,
    qr_decode_gray_with_sampling, qr_decode_gray_with_sampling_cached, qr_local_cache_memory,
    qr_tracker_decode_gray, qr_tracker_decode_gray_cached, qr_tracker_decode_gray_with_mode,
    qr_tracker_init, qr_tracker_memory, qr_tracker_reset, QR_DEFAULT_DETECTION_MODE,
    QR_DEFAULT_SAMPLING_STRATEGY, QR_MODE_DIRECT, QR_MODE_FAST, QR_SAMPLING_CENTER,
    QR_SAMPLING_CENTER_THEN_MAJORITY5,
};
#[cfg(feature = "micro-qr-detect")]
pub use gray::{
    qr_decode_micro_gray, qr_decode_micro_gray_with_budget, qr_decode_micro_gray_with_sampling,
};
use input::{bytes, bytes_mut, disjoint, range, segments_mut};

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
/// Queries recommended byte-aligned detector scratch for a mode and image dimensions.
///
/// Direct returns the minimum reported by [`qr_memory`]. Fast recommends
/// additional caller-owned storage for local-threshold memoization. Fast also
/// accepts the minimum size, falling back to uncached local decisions.
/// Returns 1 for invalid pointers, 2 for overlap, or 12 for invalid configuration,
/// mode, or dimensions. `out` is written only on success.
/// # Safety
/// `c` must point to one initialized, readable [`QrConfig`] that remains unchanged.
/// `out` must point to a live, aligned, exclusively writable `size_t`. The two
/// allocations must be disjoint. `out` need not be initialized.
#[no_mangle]
pub unsafe extern "C" fn qr_detector_scratch_bytes(
    c: *const QrConfig,
    mode: u32,
    width: usize,
    height: usize,
    out: *mut usize,
) -> i32 {
    let run = || -> Result<(), i32> {
        disjoint(&[range(c, 1)?, range(out, 1)?])?;
        let mode = detection_mode(mode)?;
        let needed = config(unsafe { c.read() })
            .scratch_bytes_for_mode(mode, width, height)
            .map_err(status)?;
        unsafe { out.write(needed) };
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
