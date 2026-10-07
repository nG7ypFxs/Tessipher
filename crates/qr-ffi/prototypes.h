/* Integer return/metadata constants; function ABI uses fixed-width integer types. */
enum qr_status {
    QR_OK = 0, QR_INVALID_ARGUMENT = 1, QR_OVERLAP = 2,
    QR_INVALID_IMAGE = 10, QR_IMAGE_TOO_LARGE = 11, QR_INVALID_CONFIG = 12,
    QR_DETECTOR_SCRATCH_TOO_SMALL = 13, QR_FINDER_CAPACITY = 14,
    QR_CANDIDATE_BUDGET = 15, QR_RETRY_BUDGET = 16, QR_NOT_FOUND = 17,
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
 * 16 retry budget; 17 no symbol; 101..110 core errors in qr_core::Error order.
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
 * Returns QR_OK, QR_INVALID_ARGUMENT, QR_OVERLAP, detector (10..17), or core (101..110) errors.
 * Corner order follows sampled-grid (0,0)/(side,0)/(side,side)/(0,side) even after transpose.
 * Coordinates are ROI-relative; there is no mirrored flag. */
int32_t qr_decode_gray(const qr_config *config,
    const uint8_t *pixels, size_t pixels_len, size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_scratch_len,
    uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);
