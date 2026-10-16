/* Generated from qr-ffi/abi_schema.rs and prototypes.h. Do not edit. */
#ifndef TESSIPHER_H
#define TESSIPHER_H
#include <stddef.h>
#include <stdint.h>
#ifndef QR_HAS_MICRO_QR
#define QR_HAS_MICRO_QR 0
#endif
#ifndef QR_HAS_MICRO_QR_DETECT
#define QR_HAS_MICRO_QR_DETECT 0
#endif
#if QR_HAS_MICRO_QR_DETECT && !QR_HAS_MICRO_QR
#error "QR_HAS_MICRO_QR_DETECT requires QR_HAS_MICRO_QR"
#endif
#ifdef __cplusplus
extern "C" {
#endif
/* ABI-1 runtime detector budgets (five u32 fields); obtain defaults with qr_default_config. */
/* Pixel reads use a finite default or qr_decode_gray_with_budget scalar argument. */
typedef struct qr_config {
    /* Finder-record capacity per pass (3..=128); default 32. */
    uint32_t max_finders;
    /* Shared finder-triple budget (1..=1000000); default 4096. */
    uint32_t max_candidates;
    /* Shared sampling-attempt budget (1..=65536); default 256; excludes core transpose retries. */
    uint32_t max_retries;
    /* Row scan step in pixels (1..=compiled maximum height); default 1. */
    uint32_t row_step;
    /* Half-width of the local threshold sample window in pixels (4..=128); default 24. */
    uint32_t local_radius;
} qr_config;
/* Compiled image limits, caller-owned storage sizes, and target ABI layout. */
typedef struct qr_memory_info {
    /* ABI version (currently 1). */
    uint32_t abi_version;
    /* Compiled maximum ROI width, in pixels. */
    uint32_t max_width;
    /* Compiled maximum ROI height, in pixels. */
    uint32_t max_height;
    /* Core scratch size sufficient for every Model 2 matrix and required by Gray8 entry points. */
    size_t core_scratch_bytes;
    /* Detector scratch size for the queried configuration. */
    size_t detector_scratch_bytes;
    /* Required alignment of both byte scratch buffers (1). */
    size_t scratch_alignment;
    /* Maximum decoded payload byte count (7089). */
    size_t max_payload_bytes;
    /* Conservative segment capacity sufficient for any supported payload (5912). */
    size_t max_segments;
    /* Size of one segment on this target, in bytes. */
    size_t segment_size;
    /* Required segment alignment on this target, in bytes. */
    size_t segment_alignment;
    /* Size of a result on this target, in bytes. */
    size_t result_size;
    /* Required result alignment on this target, in bytes. */
    size_t result_alignment;
} qr_memory_info;
/* One data or control segment in original bitstream order. */
typedef struct qr_segment {
    /* QR mode indicator: Numeric=1, Alphanumeric=2, SA=3, Byte=4, FNC1 first=5, ECI=7, Kanji=8,
     * FNC1 second=9. */
    uint32_t mode;
    /* Start of the segment's emitted bytes in the payload output. */
    size_t offset;
    /* Emitted byte count; control segments have zero length. */
    size_t len;
    /* Last ECI assignment, initially 3; controls retain all assignments in order; no transformation
     * is applied. */
    uint32_t eci;
} qr_segment;
/* Successful decoding metadata; only payload_len bytes and segment_count entries are valid. */
typedef struct qr_result {
    /* QR Model 2 version (1..=40). */
    uint32_t version;
    /* Error-correction level: 0=L, 1=M, 2=Q, 3=H. */
    uint32_t ec_level;
    /* Mask pattern (0..=7). */
    uint32_t mask;
    /* Valid prefix length of the payload output, in bytes. */
    size_t payload_len;
    /* Valid segment count, including control and empty data segments. */
    size_t segment_count;
    /* Total corrected codewords across all Reed-Solomon blocks. */
    uint32_t corrected_codewords;
    /* Selected format BCH Hamming distance (0..=3). */
    uint32_t format_distance;
    /* Selected version BCH Hamming distance (0..=3); zero for V1-6. */
    uint32_t version_distance;
    /* FNC1 header: 0=none, 1=first position, 2=second position. */
    uint32_t fnc1;
    /* Raw second-position application indicator; zero unless fnc1=2. */
    uint32_t fnc1_application_indicator;
    /* One when Structured Append metadata is present, otherwise zero. */
    uint32_t sa_present;
    /* Zero-based symbol index; meaningful only when sa_present=1. */
    uint32_t sa_index;
    /* Symbol count (1..=16); meaningful only when sa_present=1. */
    uint32_t sa_total;
    /* Original parity byte; meaningful only when sa_present=1; no reassembly is performed. */
    uint32_t sa_parity;
    /* One for an inverted-polarity detection; zero for matrix decoding. */
    uint32_t inverted;
    /* One for local-threshold detection; zero for matrix decoding. */
    uint32_t local_threshold;
    /* Finder triples examined across passes; zero for matrix decoding. */
    size_t candidates;
    /* Sampling attempts, including geometric rejection; excludes core transpose retries; zero for
     * matrix decoding. */
    size_t retries;
    /* ROI-relative x of sampled-grid boundary (0,0); zero for matrix decoding. */
    float corner0_x;
    /* ROI-relative y of sampled-grid boundary (0,0); zero for matrix decoding. */
    float corner0_y;
    /* ROI-relative x of sampled-grid boundary (side,0); zero for matrix decoding. */
    float corner1_x;
    /* ROI-relative y of sampled-grid boundary (side,0); zero for matrix decoding. */
    float corner1_y;
    /* ROI-relative x of sampled-grid boundary (side,side); zero for matrix decoding. */
    float corner2_x;
    /* ROI-relative y of sampled-grid boundary (side,side); zero for matrix decoding. */
    float corner2_y;
    /* ROI-relative x of sampled-grid boundary (0,side); zero for matrix decoding. */
    float corner3_x;
    /* ROI-relative y of sampled-grid boundary (0,side); zero for matrix decoding. */
    float corner3_y;
} qr_result;
/* Caller-owned initialized integer tracker; all-zero storage means Searching. */
/* No image/payload/pointer is retained. Do not decode uninitialized storage. */
typedef struct qr_tracker {
    /* Searching=0, Candidate=1, Tracking=2; all other tags are invalid. */
    uint32_t tag;
    /* Exactly 1 for Candidate, 0 for Searching/Tracking. */
    uint32_t misses;
    /* Saved ROI left coordinate in input-image pixels; zero when Searching. */
    size_t roi_x;
    /* Saved ROI top coordinate in input-image pixels; zero when Searching. */
    size_t roi_y;
    /* Nonzero saved ROI width; zero when Searching. */
    size_t roi_width;
    /* Nonzero saved ROI height; zero when Searching. */
    size_t roi_height;
    /* Saved input width (1..32768); zero when Searching. */
    size_t image_width;
    /* Saved input height (1..32768); zero when Searching. */
    size_t image_height;
} qr_tracker;
/* Actual decode-call work, written on success or failure after decoder invocation. */
/* Optional output remains untouched when arguments are rejected before decoding. */
typedef struct qr_stats {
    /* Actual input reads, including repeats; never exceeds the supplied limit. */
    size_t pixel_reads;
    /* Finder rows scanned across all regions/passes. */
    size_t rows;
    /* Finder run observations before detailed pattern validation. */
    size_t finder_observations;
    /* Finder triples examined across all regions/passes. */
    size_t candidates;
    /* Sampling attempts, including fallback; core transpose adds no attempt. */
    size_t retries;
    /* Threshold/polarity passes (Direct: up to 4 stateless, 8 tracked; Fast: up to 16/32). */
    size_t passes;
} qr_stats;
/* Target layout query for the new tracker and stats; ABI-1 memory layout is unchanged. */
typedef struct qr_tracker_memory_info {
    /* Size of one integer tracker in bytes. */
    size_t tracker_size;
    /* Required tracker alignment in bytes. */
    size_t tracker_alignment;
    /* Size of one stats structure in bytes. */
    size_t stats_size;
    /* Required stats alignment in bytes. */
    size_t stats_alignment;
} qr_tracker_memory_info;
/* Micro QR requirements, caller-owned storage sizes, and target ABI layouts. */
/* This additive query does not change qr_memory_info or ABI version 1. */
typedef struct qr_micro_memory_info {
    /* ABI version of the original C interface (currently 1). */
    uint32_t abi_version;
    /* Compiled maximum ROI width, in pixels. */
    uint32_t max_width;
    /* Compiled maximum ROI height, in pixels. */
    uint32_t max_height;
    /* Scratch bytes required by qr_decode_micro_matrix (184). */
    size_t matrix_core_scratch_bytes;
    /* Core scratch bytes required by Micro QR Gray8 entry points (184), or zero when unavailable.
     */
    size_t image_core_scratch_bytes;
    /* Detector scratch bytes for the queried configuration, or zero when unavailable. */
    size_t detector_scratch_bytes;
    /* Required alignment of all byte scratch buffers (1). */
    size_t scratch_alignment;
    /* Largest decoded Micro QR payload, in bytes (35). */
    size_t max_payload_bytes;
    /* Conservative segment capacity sufficient for every Micro QR symbol (16). */
    size_t max_segments;
    /* Size of one qr_segment on this target, in bytes. */
    size_t segment_size;
    /* Required qr_segment alignment on this target, in bytes. */
    size_t segment_alignment;
    /* Size of one qr_micro_result on this target, in bytes. */
    size_t result_size;
    /* Required qr_micro_result alignment on this target, in bytes. */
    size_t result_alignment;
    /* Size of one qr_stats on this target, in bytes. */
    size_t stats_size;
    /* Required qr_stats alignment on this target, in bytes. */
    size_t stats_alignment;
} qr_micro_memory_info;
/* Successful Micro QR decoding metadata. */
typedef struct qr_micro_result {
    /* Micro QR version (1=M1, 2=M2, 3=M3, 4=M4). */
    uint32_t version;
    /* Error correction: 0=L, 1=M, 2=Q, or QR_MICRO_EC_NONE for M1. */
    uint32_t ec_level;
    /* Micro QR mask pattern (0..=3). */
    uint32_t mask;
    /* Valid prefix length of the payload output, in bytes. */
    size_t payload_len;
    /* Valid segment count. */
    size_t segment_count;
    /* Corrected codewords in the symbol's single Reed-Solomon block. */
    uint32_t corrected_codewords;
    /* Selected format BCH Hamming distance (0..=3). */
    uint32_t format_distance;
    /* Clockwise quarter turns used to orient the symbol (0..=3). */
    uint32_t rotation;
    /* One when core decoding required reflection after rotation. */
    uint32_t mirrored;
    /* One for an inverted-polarity detection; zero for matrix decoding. */
    uint32_t inverted;
    /* One for local-threshold detection; zero for matrix decoding. */
    uint32_t local_threshold;
    /* Finder candidates examined; zero for matrix decoding. */
    size_t candidates;
    /* Sampling attempts; zero for matrix decoding. */
    size_t retries;
    /* ROI-relative x of sampled-grid boundary (0,0); zero for matrix decoding. */
    float corner0_x;
    /* ROI-relative y of sampled-grid boundary (0,0); zero for matrix decoding. */
    float corner0_y;
    /* ROI-relative x of sampled-grid boundary (side,0); zero for matrix decoding. */
    float corner1_x;
    /* ROI-relative y of sampled-grid boundary (side,0); zero for matrix decoding. */
    float corner1_y;
    /* ROI-relative x of sampled-grid boundary (side,side); zero for matrix decoding. */
    float corner2_x;
    /* ROI-relative y of sampled-grid boundary (side,side); zero for matrix decoding. */
    float corner2_y;
    /* ROI-relative x of sampled-grid boundary (0,side); zero for matrix decoding. */
    float corner3_x;
    /* ROI-relative y of sampled-grid boundary (0,side); zero for matrix decoding. */
    float corner3_y;
} qr_micro_result;
/* Integer return/metadata constants; function ABI uses fixed-width integer types. */
enum qr_status {
    QR_OK = 0,
    QR_INVALID_ARGUMENT = 1,
    QR_OVERLAP = 2,
    QR_INVALID_IMAGE = 10,
    QR_IMAGE_TOO_LARGE = 11,
    QR_INVALID_CONFIG = 12,
    QR_DETECTOR_SCRATCH_TOO_SMALL = 13,
    QR_FINDER_CAPACITY = 14,
    QR_CANDIDATE_BUDGET = 15,
    QR_RETRY_BUDGET = 16,
    QR_NOT_FOUND = 17,
    QR_PIXEL_BUDGET = 18,
    QR_INVALID_MATRIX = 101,
    QR_CORE_SCRATCH_TOO_SMALL = 102,
    QR_FORMAT = 103,
    QR_VERSION = 104,
    QR_REED_SOLOMON = 105,
    QR_TRUNCATED = 106,
    QR_UNSUPPORTED_MODE = 107,
    QR_MALFORMED_PAYLOAD = 108,
    QR_OUTPUT_TOO_SMALL = 109,
    QR_SEGMENTS_TOO_SMALL = 110
};
#if QR_HAS_MICRO_QR
enum qr_micro_ec_level {
    QR_MICRO_EC_L = 0,
    QR_MICRO_EC_M = 1,
    QR_MICRO_EC_Q = 2
};
#define QR_MICRO_EC_NONE UINT32_MAX
#endif
enum qr_ec_level {
    QR_EC_L = 0,
    QR_EC_M = 1,
    QR_EC_Q = 2,
    QR_EC_H = 3
};
enum qr_mode {
    QR_NUMERIC = 1,
    QR_ALPHANUMERIC = 2,
    QR_STRUCTURED_APPEND = 3,
    QR_BYTE = 4,
    QR_FNC1_FIRST = 5,
    QR_ECI = 7,
    QR_KANJI = 8,
    QR_FNC1_SECOND = 9
};
/* Status: 0 success; 1 invalid numeric pointer/length/alignment argument;
 * 2 overlapping buffers; 10 invalid image; 11 image too large; 12 invalid config;
 * 13 detector scratch shortage; 14 finder capacity; 15 candidate budget;
 * 16 retry budget; 17 no symbol; 18 pixel-read budget; 101..110 core errors in
 * tessipher::core::Error order. ECC values: 0=L, 1=M, 2=Q, 3=H. Mode values are QR mode indicator
 * numbers. FNC1: 0=none, 1=first, 2=second. SA index is zero based, total is 1..16. qr_segment.eci
 * is the last ECI number encountered, initially 3, not an active set. ECI control segments retain
 * every assignment in order for caller interpretation. No charset, encryption, or compression
 * transformation is performed. Each nonempty span must lie in one live allocation, aligned for its
 * type. All nonempty spans, including inputs, must be mutually disjoint. Inputs/config are
 * initialized and unchanged for the duration of the call. Pointer contracts: see docs/ffi.md.
 * Scratch/payload/segment/result outputs must be exclusive writable allocations, disjoint from all
 * input and each other. Null is allowed only for zero-length byte/segment spans. Config/result are
 * mandatory. Matrix bits are packed row-major, MSB first, without row padding. Output storage need
 * not be initialized; no concurrent access is permitted. Numeric checks cannot prove allocation
 * size, validity, lifetime, or exclusivity. On error output buffers are unspecified; result is
 * written only on success. Read only result.payload_len bytes and result.segment_count entries on
 * success.
 */
/* Writes defaults: finders32/candidates4096/retries256/row_step1/local_radius24.
 * Returns QR_OK or QR_INVALID_ARGUMENT. out is mandatory and exclusively writable. */
int32_t qr_default_config(qr_config *out);
/* Queries the linked library limits and target layouts for a validated config.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, or QR_INVALID_CONFIG.
 * config is mandatory/readable; out is mandatory/writable and written only on success. */
int32_t qr_memory(const qr_config *config, qr_memory_info *out);

/* Queries Model 2 matrix core scratch for version 1..40 (460 bytes at V1).
 * Returns QR_VERSION for an invalid version, QR_INVALID_ARGUMENT for invalid out.
 * out is written only on success and must be aligned, live, writable size_t storage.
 * Scratch alignment is 1. Image detection and tracking instead require qr_memory. */
int32_t qr_matrix_scratch_bytes(uint32_t version, size_t *out);
/* side is 17 + 4 * version (version 1..40); bits have no quiet-zone border.
 * Normal orientation is tried first, then the transpose after content errors.
 * Scratch requires qr_matrix_scratch_bytes(version), or qr_memory_info.core_scratch_bytes
 * for any version; segment_capacity <= max_segments.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, or a core error (101..110).
 * Detection-only result fields are zero. */
int32_t qr_decode_matrix(const uint8_t *bits, size_t bits_len, size_t side, uint8_t *scratch,
                         size_t scratch_len, uint8_t *payload, size_t payload_len,
                         qr_segment *segments, size_t segment_capacity, qr_result *result);
#if QR_HAS_MICRO_QR
/* Queries Micro QR limits, scratch sizes, and additive type layouts.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, or QR_INVALID_CONFIG.
 * config is mandatory/readable; out is mandatory/writable and success-only.
 * The declaration and linked symbol require QR_HAS_MICRO_QR=1. */
int32_t qr_micro_memory(const qr_config *config, qr_micro_memory_info *out);
/* Decodes M1-M4 packed matrices. side is 11, 13, 15, or 17; no quiet zone.
 * Requires matrix_core_scratch_bytes; segment_capacity <= max_segments.
 * M1 reports QR_MICRO_EC_NONE. Orientation metadata reports any core rotation/reflection.
 * This declaration and its linked symbol require QR_HAS_MICRO_QR=1. */
int32_t qr_decode_micro_matrix(const uint8_t *bits, size_t bits_len, size_t side, uint8_t *scratch,
                               size_t scratch_len, uint8_t *payload, size_t payload_len,
                               qr_segment *segments, size_t segment_capacity,
                               qr_micro_result *result);
#endif
#if QR_HAS_MICRO_QR_DETECT
/* Micro QR Gray8 detection has the same image, pointer, budget and sampling contracts as
 * the corresponding Model 2 calls. Query qr_micro_memory for both scratch lengths.
 * The detector is bounded and affine; strong projective distortion may return QR_NOT_FOUND.
 * These declarations and linked symbols require QR_HAS_MICRO_QR_DETECT=1. */
int32_t qr_decode_micro_gray(const qr_config *config, const uint8_t *pixels, size_t pixels_len,
                             size_t width, size_t height, size_t stride, uint8_t *detector_scratch,
                             size_t detector_scratch_len, uint8_t *core_scratch,
                             size_t core_scratch_len, uint8_t *payload, size_t payload_len,
                             qr_segment *segments, size_t segment_capacity,
                             qr_micro_result *result);
int32_t qr_decode_micro_gray_with_budget(const qr_config *config, size_t max_pixel_reads,
                                         const uint8_t *pixels, size_t pixels_len, size_t width,
                                         size_t height, size_t stride, uint8_t *detector_scratch,
                                         size_t detector_scratch_len, uint8_t *core_scratch,
                                         size_t core_scratch_len, uint8_t *payload,
                                         size_t payload_len, qr_segment *segments,
                                         size_t segment_capacity, qr_micro_result *result);
/* Explicit sampling with optional stats. Stats are written only after the decoder starts;
 * argument, configuration, and image rejection preserve them. */
int32_t qr_decode_micro_gray_with_sampling(
    const qr_config *config, size_t max_pixel_reads, uint32_t strategy, const uint8_t *pixels,
    size_t pixels_len, size_t width, size_t height, size_t stride, uint8_t *detector_scratch,
    size_t detector_scratch_len, uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload,
    size_t payload_len, qr_segment *segments, size_t segment_capacity, qr_micro_result *result,
    qr_stats *stats);
#endif
/* Decodes the first symbol in a Gray8 ROI; pixels starts at the ROI top-left.
 * Nonzero width/height, stride >= width, and (height-1)*stride+width bytes are required.
 * Last-row trailing padding is unnecessary; input is never copied.
 * Query qr_memory for compiled limits and both scratch lengths; segment_capacity <= max_segments.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, detector (10..18), or core (101..110) errors.
 * Corner order follows sampled-grid (0,0)/(side,0)/(side,side)/(0,side) even after transpose.
 * Coordinates are ROI-relative; there is no mirrored flag.
 * Uses a finite limit of 64000000 actual input pixel reads shared across the call. */
int32_t qr_decode_gray(const qr_config *config, const uint8_t *pixels, size_t pixels_len,
                       size_t width, size_t height, size_t stride, uint8_t *detector_scratch,
                       size_t detector_scratch_len, uint8_t *core_scratch, size_t core_scratch_len,
                       uint8_t *payload, size_t payload_len, qr_segment *segments,
                       size_t segment_capacity, qr_result *result);

/* Same pointer, buffer, result and error contract as qr_decode_gray.
 * max_pixel_reads is 1..SIZE_MAX; zero returns QR_INVALID_CONFIG.
 * Every actual input read, including min/max, repeats and all passes, is charged.
 * Direct reuses summaries for local row scans; uncached local decisions cost
 * 26 reads, including clamped duplicate samples.
 * Returns QR_PIXEL_BUDGET before another read would exceed the limit.
 * The limit bounds pixel work, not total cycles or caller-buffer initialization.
 * The ABI-1 qr_config/qr_result/qr_memory_info layouts are unchanged. */
int32_t qr_decode_gray_with_budget(const qr_config *config, size_t max_pixel_reads,
                                   const uint8_t *pixels, size_t pixels_len, size_t width,
                                   size_t height, size_t stride, uint8_t *detector_scratch,
                                   size_t detector_scratch_len, uint8_t *core_scratch,
                                   size_t core_scratch_len, uint8_t *payload, size_t payload_len,
                                   qr_segment *segments, size_t segment_capacity,
                                   qr_result *result);

/* Sampling arguments are uint32_t; unknown values return QR_INVALID_CONFIG.
 * Existing Gray8 entry points use QR_DEFAULT_SAMPLING_STRATEGY; no hidden tracking.
 * Explicit Center preserves the earlier sampling/limited-budget behavior. */
enum qr_sampling_strategy {
    QR_SAMPLING_CENTER = 0,
    QR_SAMPLING_CENTER_THEN_MAJORITY5 = 1,
    QR_DEFAULT_SAMPLING_STRATEGY = QR_SAMPLING_CENTER_THEN_MAJORITY5
};
/* Modes affect Model 2 Gray8 detection only. All accept qr_memory minimum scratch;
 * Fast can use additional caller RAM reported by qr_detector_scratch_bytes.
 * Non-cached entry points default to Direct. Five-row cached APIs retain their
 * dedicated cache path and do not select a mode.
 * Fast tries coarse scanning before
 * full scanning and can select a different first symbol in multi-symbol images.
 * Mode arguments use uint32_t; other values (including retired value 1)
 * return QR_INVALID_CONFIG before
 * modifying buffers, stats, result, or tracker. */
enum qr_detection_mode {
    QR_MODE_DIRECT = 0,
    QR_MODE_FAST = 2,
    QR_DEFAULT_DETECTION_MODE = QR_MODE_DIRECT
};
/* Recommended byte-aligned detector scratch for the mode and image dimensions.
 * Direct returns qr_memory's minimum; Fast adds ceil(width*height/4)
 * bytes for local-threshold memoization. Minimum-sized Fast buffers remain valid.
 * Config and out must be live, aligned, disjoint allocations. Writes out only on
 * success. Invalid modes, dimensions or config return QR_INVALID_CONFIG.
 * Pointer/overlap errors follow qr_memory. */
int32_t qr_detector_scratch_bytes(const qr_config *config, uint32_t mode, size_t width,
                                  size_t height, size_t *out);
enum qr_tracker_tag {
    QR_TRACKER_SEARCHING = 0,
    QR_TRACKER_CANDIDATE = 1,
    QR_TRACKER_TRACKING = 2
};
/* New layout query; qr_memory_info and ABI version 1 are unchanged. */
int32_t qr_tracker_memory(qr_tracker_memory_info *out);
/* Write-only initialization/reset to canonical all-zero Searching.
 * out may be uninitialized but must be a live aligned exclusive writable allocation.
 * Returns QR_OK or QR_INVALID_ARGUMENT; decode must never read uninitialized state. */
int32_t qr_tracker_init(qr_tracker *out);
int32_t qr_tracker_reset(qr_tracker *out);
/* Like qr_decode_gray_with_budget, with explicit sampling and optional stats.
 * NULL stats is allowed. Nonnull stats must be aligned/writable/disjoint from all spans.
 * Stats are written on success/failure after decoding starts; pre-decoder argument,
 * configuration, state or image rejection leaves stats untouched. Result is success-only.
 * Five-point fallback costs a retry and up to 5 global or 130 local reads per module. */
int32_t qr_decode_gray_with_sampling(const qr_config *config, size_t max_pixel_reads,
                                     uint32_t strategy, const uint8_t *pixels, size_t pixels_len,
                                     size_t width, size_t height, size_t stride,
                                     uint8_t *detector_scratch, size_t detector_scratch_len,
                                     uint8_t *core_scratch, size_t core_scratch_len,
                                     uint8_t *payload, size_t payload_len, qr_segment *segments,
                                     size_t segment_capacity, qr_result *result, qr_stats *stats);
/* Caller-owned tracker, explicit sampling and shared frame pixel limit.
 * Initialize via qr_tracker_init or all-zero initialization before use.
 * Every tracker/state/stats/config/input/output span must be mutually disjoint.
 * Unknown tags, noncanonical Searching fields, invalid misses or saved bounds return
 * QR_INVALID_ARGUMENT without modifying tracker/result/stats. Candidate misses=1;
 * Tracking misses=0. Saved dimensions must be nonzero, within compiled maxima, and contain the
 * nonempty ROI. Current width/height changes force a full search; same-size crop/scale changes
 * require reset. Ordinary ROI miss triggers same-frame full search sharing all counters and
 * budgets. First complete frame miss keeps Candidate; second expires it. No image/payload is
 * retained. Only success/complete recognition miss update state. All terminal errors preserve
 * state. Result corners are relative to the complete supplied image, not the internal tracked ROI.
 */
int32_t qr_tracker_decode_gray(qr_tracker *tracker, const qr_config *config, size_t max_pixel_reads,
                               uint32_t strategy, const uint8_t *pixels, size_t pixels_len,
                               size_t width, size_t height, size_t stride,
                               uint8_t *detector_scratch, size_t detector_scratch_len,
                               uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload,
                               size_t payload_len, qr_segment *segments, size_t segment_capacity,
                               qr_result *result, qr_stats *stats);

/* Explicit mode variants accept qr_memory minimum scratch and the same pointer, budget,
 * sampling, stats, and tracker-state contracts as their mode-unspecified counterparts.
 * qr_detector_scratch_bytes recommends additional Fast scratch for memoization.
 * Direct reuses detector scratch; Fast tries coarse scanning and nearby global thresholds.
 * Each added threshold pass probes at most two matrix samplings within shared budgets.
 * Stats.passes is at most 16 stateless or 32 tracked in Fast mode. */
int32_t qr_decode_gray_with_mode(const qr_config *config, size_t max_pixel_reads, uint32_t mode,
                                 uint32_t strategy, const uint8_t *pixels, size_t pixels_len,
                                 size_t width, size_t height, size_t stride,
                                 uint8_t *detector_scratch, size_t detector_scratch_len,
                                 uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload,
                                 size_t payload_len, qr_segment *segments, size_t segment_capacity,
                                 qr_result *result, qr_stats *stats);
int32_t qr_tracker_decode_gray_with_mode(
    qr_tracker *tracker, const qr_config *config, size_t max_pixel_reads, uint32_t mode,
    uint32_t strategy, const uint8_t *pixels, size_t pixels_len, size_t width, size_t height,
    size_t stride, uint8_t *detector_scratch, size_t detector_scratch_len, uint8_t *core_scratch,
    size_t core_scratch_len, uint8_t *payload, size_t payload_len, qr_segment *segments,
    size_t segment_capacity, qr_result *result, qr_stats *stats);

/* Optional five-row local Gray8 cache in caller-owned internal RAM (alignment 1).
 * Invalid config, zero width or width over compiled maximum returns QR_INVALID_CONFIG.
 * config/out mandatory, aligned and disjoint; out written only on success. */
int32_t qr_local_cache_memory(const qr_config *config, size_t width, size_t *out);
/* Cached variants: cache requires 5 * width bytes. All supplied cache_len bytes
 * must be exclusive and disjoint from every other supplied span. Excess bytes
 * remain untouched. Tags reset each invocation and ROI/full retry. Pixel-read
 * stats and budget count input reads during fills, exclude internal RAM hits.
 * Horizontal finder scans fill rows; irregular helpers reuse hits, and read
 * misses directly without replacing scan rows. Full-row prefetch can exhaust
 * a finite budget earlier than uncached sampling.
 * Stats is optional and follows the uncached functions' contract. */
int32_t qr_decode_gray_with_sampling_cached(
    const qr_config *config, size_t max_pixel_reads, uint32_t strategy, const uint8_t *pixels,
    size_t pixels_len, size_t width, size_t height, size_t stride, uint8_t *detector_scratch,
    size_t detector_scratch_len, uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload,
    size_t payload_len, qr_segment *segments, size_t segment_capacity, qr_result *result,
    qr_stats *stats, uint8_t *cache, size_t cache_len);
int32_t qr_tracker_decode_gray_cached(
    qr_tracker *tracker, const qr_config *config, size_t max_pixel_reads, uint32_t strategy,
    const uint8_t *pixels, size_t pixels_len, size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_scratch_len, uint8_t *core_scratch,
    size_t core_scratch_len, uint8_t *payload, size_t payload_len, qr_segment *segments,
    size_t segment_capacity, qr_result *result, qr_stats *stats, uint8_t *cache, size_t cache_len);

#ifdef __cplusplus
}
#endif
#endif
