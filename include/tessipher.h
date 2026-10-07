/* Generated from qr-ffi/abi_schema.rs and prototypes.h. Do not edit. */
#ifndef TESSIPHER_H
#define TESSIPHER_H
#include <stddef.h>
#include <stdint.h>
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
    /* Core scratch size, independent of the matrix version. */
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
    /* QR mode indicator: Numeric=1, Alphanumeric=2, SA=3, Byte=4, FNC1 first=5, ECI=7, Kanji=8, FNC1 second=9. */
    uint32_t mode;
    /* Start of the segment's emitted bytes in the payload output. */
    size_t offset;
    /* Emitted byte count; control segments have zero length. */
    size_t len;
    /* Last ECI assignment, initially 3; controls retain all assignments in order; no transformation is applied. */
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
    /* Sampling attempts, including geometric rejection; excludes core transpose retries; zero for matrix decoding. */
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
/* Integer return/metadata constants; function ABI uses fixed-width integer types. */
enum qr_status {
    QR_OK = 0, QR_INVALID_ARGUMENT = 1, QR_OVERLAP = 2,
    QR_INVALID_IMAGE = 10, QR_IMAGE_TOO_LARGE = 11, QR_INVALID_CONFIG = 12,
    QR_DETECTOR_SCRATCH_TOO_SMALL = 13, QR_FINDER_CAPACITY = 14,
    QR_CANDIDATE_BUDGET = 15, QR_RETRY_BUDGET = 16, QR_NOT_FOUND = 17, QR_PIXEL_BUDGET = 18,
    QR_INVALID_MATRIX = 101, QR_CORE_SCRATCH_TOO_SMALL = 102, QR_FORMAT = 103,
    QR_VERSION = 104, QR_REED_SOLOMON = 105, QR_TRUNCATED = 106,
    QR_UNSUPPORTED_MODE = 107, QR_MALFORMED_PAYLOAD = 108,
    QR_OUTPUT_TOO_SMALL = 109, QR_SEGMENTS_TOO_SMALL = 110
};
enum qr_ec_level { QR_EC_L = 0, QR_EC_M = 1, QR_EC_Q = 2, QR_EC_H = 3 };
enum qr_mode {
    QR_NUMERIC = 1, QR_ALPHANUMERIC = 2, QR_STRUCTURED_APPEND = 3,
    QR_BYTE = 4, QR_FNC1_FIRST = 5, QR_ECI = 7, QR_KANJI = 8, QR_FNC1_SECOND = 9
};
/* Status: 0 success; 1 invalid numeric pointer/length/alignment argument;
 * 2 overlapping buffers; 10 invalid image; 11 image too large; 12 invalid config;
 * 13 detector scratch shortage; 14 finder capacity; 15 candidate budget;
 * 16 retry budget; 17 no symbol; 18 pixel-read budget; 101..110 core errors in qr_core::Error order.
 * ECC values: 0=L, 1=M, 2=Q, 3=H. Mode values are QR mode indicator numbers.
 * FNC1: 0=none, 1=first, 2=second. SA index is zero based, total is 1..16.
 * qr_segment.eci is the last ECI number encountered, initially 3, not an active set.
 * ECI control segments retain every assignment in order for caller interpretation.
 * No charset, encryption, or compression transformation is performed.
 * Each nonempty span must lie in one live allocation, aligned for its type.
 * All nonempty spans, including inputs, must be mutually disjoint.
 * Inputs/config are initialized and unchanged for the duration of the call.
 * Pointer contracts: see docs/ffi.md. Scratch/payload/segment/result outputs
 * must be exclusive writable allocations, disjoint from all input and each other.
 * Null is allowed only for zero-length byte/segment spans. Config/result are mandatory.
 * Matrix bits are packed row-major, MSB first, without row padding.
 * Output storage need not be initialized; no concurrent access is permitted.
 * Numeric checks cannot prove allocation size, validity, lifetime, or exclusivity.
 * On error output buffers are unspecified; result is written only on success.
 * Read only result.payload_len bytes and result.segment_count entries on success.
 */
/* Writes defaults: finders32/candidates4096/retries256/row_step1/local_radius24.
 * Returns QR_OK or QR_INVALID_ARGUMENT. out is mandatory and exclusively writable. */
int32_t qr_default_config(qr_config *out);
/* Queries the linked library limits and target layouts for a validated config.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, or QR_INVALID_CONFIG.
 * config is mandatory/readable; out is mandatory/writable and written only on success. */
int32_t qr_memory(const qr_config *config, qr_memory_info *out);
/* side is 17 + 4 * version (version 1..40); bits have no quiet-zone border.
 * Normal orientation is tried first, then the transpose after content errors.
 * Scratch requires qr_memory_info.core_scratch_bytes; segment_capacity <= max_segments.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, or a core error (101..110).
 * Detection-only result fields are zero. */
int32_t qr_decode_matrix(const uint8_t *bits, size_t bits_len, size_t side,
    uint8_t *scratch, size_t scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);
/* Decodes the first symbol in a Gray8 ROI; pixels starts at the ROI top-left.
 * Nonzero width/height, stride >= width, and (height-1)*stride+width bytes are required.
 * Last-row trailing padding is unnecessary; input is never copied.
 * Query qr_memory for compiled limits and both scratch lengths; segment_capacity <= max_segments.
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, detector (10..18), or core (101..110) errors.
 * Corner order follows sampled-grid (0,0)/(side,0)/(side,side)/(0,side) even after transpose.
 * Coordinates are ROI-relative; there is no mirrored flag.
 * Uses a finite limit of 64000000 actual input pixel reads shared across the call. */
int32_t qr_decode_gray(const qr_config *config,
    const uint8_t *pixels, size_t pixels_len, size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_scratch_len,
    uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);

/* Same pointer, buffer, result and error contract as qr_decode_gray.
 * max_pixel_reads is 1..SIZE_MAX; zero returns QR_INVALID_CONFIG.
 * Every actual input read, including min/max, repeats and all passes, is charged.
 * Local threshold decisions cost 26 reads, including clamped duplicate samples.
 * Returns QR_PIXEL_BUDGET before another read would exceed the limit.
 * The limit bounds pixel work, not total cycles or caller-buffer initialization.
 * The ABI-1 qr_config/qr_result/qr_memory_info layouts are unchanged. */
int32_t qr_decode_gray_with_budget(const qr_config *config, size_t max_pixel_reads,
    const uint8_t *pixels, size_t pixels_len, size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_scratch_len,
    uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);

#ifdef __cplusplus
}
#endif
#endif
