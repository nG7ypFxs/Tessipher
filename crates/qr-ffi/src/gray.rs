//! Gray8 decoding, caller-owned tracker state, and optional local-cache entry points.

use super::input::{bytes, bytes_mut, disjoint, range, segments_mut, Range};
use super::{
    config, detect, qcore, result, status, QrConfig, QrResult, QrSegment, QrStats, QrTracker,
    QrTrackerMemory, MAX_HEIGHT, MAX_WIDTH,
};
#[cfg(feature = "micro-qr-detect")]
use super::{micro_result, QrMicroResult, MICRO_MAX_SEGMENTS};
use core::{
    mem::{align_of, size_of},
    ptr,
};

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
/// [`crate::qr_micro_memory`] for the detector and image-core scratch sizes.
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
/// [`MAX_HEIGHT`]; query [`crate::qr_memory`] for scratch sizes and alignment.
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
/// reads. Direct reuses local scan summaries; uncached local decisions cost 26
/// reads. The default entry point uses
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

/// Optimized scanning and local-threshold summaries in existing detector scratch.
pub const QR_MODE_DIRECT: u32 = 0;
/// Direct processing with a coarse search before the full-resolution fallback.
pub const QR_MODE_FAST: u32 = 2;
/// Detection mode used by non-cached Gray8 C entry points.
pub const QR_DEFAULT_DETECTION_MODE: u32 = QR_MODE_DIRECT;

pub(super) fn detection_mode(n: u32) -> Result<detect::DetectionMode, i32> {
    match n {
        QR_MODE_DIRECT => Ok(detect::DetectionMode::Direct),
        QR_MODE_FAST => Ok(detect::DetectionMode::Fast),
        _ => Err(12),
    }
}
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
    mode: u32,
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
                None => Range::EMPTY,
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
                None => Range::EMPTY,
            },
        ])?;
        let strategy = sampling(a.strategy)?;
        let mode = detection_mode(a.mode)?;
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
            (Some(t), None) => t.decode_with_mode(
                image, &cfg, mode, strategy, ds, cs, output, segments, &mut stats,
            ),
            (None, None) => detect::decode_with_mode(
                image, &cfg, mode, strategy, ds, cs, output, segments, &mut stats,
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
            mode: QR_DEFAULT_DETECTION_MODE,
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

/// Decodes Gray8 with explicit detection mode, sampling, pixel budget, and optional stats.
/// Modes 0/2 select Direct/Fast; other values (including retired value 1) return
/// 12 before writable storage is initialized. Non-cached entry points default to Direct.
/// Fast first tries coarse scanning, then full scanning after recognition failure;
/// its first successful symbol can differ when an image contains multiple symbols.
/// All modes accept the minimum scratch lengths reported by [`crate::qr_memory`].
/// [`crate::qr_detector_scratch_bytes`] recommends extra Fast scratch for the supplied
/// image dimensions; the extra storage avoids repeated local-threshold work.
/// # Safety
/// All pointer and exclusive-access requirements of [`qr_decode_gray_with_sampling`] apply.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_decode_gray_with_mode(
    c: *const QrConfig,
    max_pixel_reads: usize,
    mode: u32,
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
            mode,
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
            mode: QR_MODE_DIRECT,
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
            mode: QR_DEFAULT_DETECTION_MODE,
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

/// Decodes one tracked frame with an explicit detection mode.
/// Unknown modes return 12 before modifying tracker, result, stats, or buffers.
/// Mode selection and scratch requirements match [`qr_decode_gray_with_mode`].
/// Terminal decoder errors preserve tracker state as in [`qr_tracker_decode_gray`].
/// # Safety
/// All pointer, initialization, and exclusive-access requirements of
/// [`qr_tracker_decode_gray`] apply.
#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub unsafe extern "C" fn qr_tracker_decode_gray_with_mode(
    tracker: *mut QrTracker,
    c: *const QrConfig,
    max_pixel_reads: usize,
    mode: u32,
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
            mode,
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
            mode: QR_MODE_DIRECT,
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
mod tests;
