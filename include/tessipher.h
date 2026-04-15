/* Generated from qr-ffi/abi_schema.rs and prototypes.h. Do not edit. */
#ifndef TESSIPHER_H
#define TESSIPHER_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct qr_config {
    uint32_t max_finders;
    uint32_t max_candidates;
    uint32_t max_retries;
    uint32_t row_step;
    uint32_t local_radius;
} qr_config;
typedef struct qr_memory_info {
    uint32_t abi_version;
    uint32_t max_width;
    uint32_t max_height;
    size_t core_scratch_bytes;
    size_t detector_scratch_bytes;
    size_t scratch_alignment;
    size_t max_payload_bytes;
    size_t max_segments;
    size_t segment_size;
    size_t segment_alignment;
    size_t result_size;
    size_t result_alignment;
} qr_memory_info;
typedef struct qr_segment {
    uint32_t mode;
    size_t offset;
    size_t len;
    uint32_t eci;
} qr_segment;
typedef struct qr_result {
    uint32_t version;
    uint32_t ec_level;
    uint32_t mask;
    size_t payload_len;
    size_t segment_count;
    uint32_t corrected_codewords;
    uint32_t format_distance;
    uint32_t version_distance;
    uint32_t fnc1;
    uint32_t fnc1_application_indicator;
    uint32_t sa_present;
    uint32_t sa_index;
    uint32_t sa_total;
    uint32_t sa_parity;
    uint32_t inverted;
    uint32_t local_threshold;
    size_t candidates;
    size_t retries;
    float corner0_x;
    float corner0_y;
    float corner1_x;
    float corner1_y;
    float corner2_x;
    float corner2_y;
    float corner3_x;
    float corner3_y;
} qr_result;
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
/* Status: 0 success; 1 invalid pointer/length/alignment/config argument;
 * 2 overlapping buffers; 10 invalid image; 11 image too large; 12 invalid config;
 * 13 detector scratch shortage; 14 finder capacity; 15 candidate budget;
 * 16 retry budget; 17 no symbol; 101..110 core errors in qr_core::Error order.
 * ECC values: 0=L, 1=M, 2=Q, 3=H. Mode values are QR mode indicator numbers.
 * FNC1: 0=none, 1=first, 2=second. SA index is zero based, total is 1..16.
 * qr_segment.eci is the last ECI number encountered, initially 3, not an active set.
 * ECI control segments retain every assignment in order for caller interpretation.
 * No charset, encryption, or compression transformation is performed.
 * Pointer contracts: see docs/ffi.md. Scratch/payload/segment/result outputs
 * must be exclusive writable allocations, disjoint from all input and each other.
 * Null is allowed only for zero-length byte/segment spans. Config/result are mandatory.
 * Matrix bits are packed row-major, MSB first, without row padding.
 * On error output buffers are unspecified; result is written only on success.
 */
int32_t qr_default_config(qr_config *out);
int32_t qr_memory(const qr_config *config, qr_memory_info *out);
int32_t qr_decode_matrix(const uint8_t *bits, size_t bits_len, size_t side,
    uint8_t *scratch, size_t scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);
int32_t qr_decode_gray(const qr_config *config,
    const uint8_t *pixels, size_t pixels_len, size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_scratch_len,
    uint8_t *core_scratch, size_t core_scratch_len, uint8_t *payload, size_t payload_len,
    qr_segment *segments, size_t segment_capacity, qr_result *result);

#ifdef __cplusplus
}
#endif
#endif
