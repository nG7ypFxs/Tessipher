#include <zephyr/ztest.h>
#include "tessipher.h"
#include <stdalign.h>
#include <string.h>

static const uint8_t matrix[] = {
#include "v1.inc"
};
static uint8_t core_scratch[7077];
static uint8_t detector_scratch[4557];
static uint8_t payload[16];
static qr_segment segments[4];
static uint8_t pixels[116 * 116];
static qr_config config;
static qr_memory_info memory;

static void *setup(void) {
    zassert_equal(qr_default_config(&config), QR_OK);
    zassert_equal(qr_memory(&config, &memory), QR_OK);
    zassert_equal(memory.abi_version, 1);
    zassert_true(memory.max_width >= 116 && memory.max_height >= 116);
    zassert_true(memory.core_scratch_bytes <= sizeof(core_scratch));
    zassert_true(memory.detector_scratch_bytes <= sizeof(detector_scratch));
    zassert_equal(memory.scratch_alignment, 1);
    zassert_equal(memory.segment_size, sizeof(qr_segment));
    zassert_equal(memory.segment_alignment, alignof(qr_segment));
    zassert_equal(memory.result_size, sizeof(qr_result));
    zassert_equal(memory.result_alignment, alignof(qr_result));
    zassert_true(ARRAY_SIZE(segments) <= memory.max_segments);
    zassert_true(sizeof(payload) <= memory.max_payload_bytes);
    zassert_equal(sizeof(matrix), 56);

    /* Four-pixel modules and a four-module white quiet zone. */
    memset(pixels, 255, sizeof(pixels));
    for (size_t y = 0; y < 21; ++y) {
        for (size_t x = 0; x < 21; ++x) {
            size_t bit = y * 21 + x;
            if ((matrix[bit / 8] & (0x80 >> (bit % 8))) == 0)
                continue;
            for (size_t dy = 0; dy < 4; ++dy) {
                for (size_t dx = 0; dx < 4; ++dx) {
                    pixels[((y + 4) * 4 + dy) * 116 + (x + 4) * 4 + dx] = 0;
                }
            }
        }
    }
    return NULL;
}

static void check_payload(const qr_result *result) {
    zassert_equal(result->version, 1);
    zassert_equal(result->ec_level, QR_EC_H);
    zassert_equal(result->mask, 3);
    zassert_equal(result->payload_len, 4);
    zassert_equal(result->segment_count, 1);
    zassert_mem_equal(payload, "TEST", 4);
    zassert_equal(segments[0].mode, QR_BYTE);
    zassert_equal(segments[0].offset, 0);
    zassert_equal(segments[0].len, 4);
    zassert_equal(segments[0].eci, 3);
}

ZTEST(ffi, test_matrix) {
    qr_result result;
    zassert_equal(qr_decode_matrix(matrix, sizeof(matrix), 21, core_scratch, sizeof(core_scratch),
                                   payload, sizeof(payload), segments, ARRAY_SIZE(segments),
                                   &result),
                  QR_OK);
    check_payload(&result);
    zassert_equal(result.inverted, 0);
    zassert_equal(result.local_threshold, 0);
    zassert_equal(result.candidates, 0);
    zassert_equal(result.retries, 0);
}

ZTEST(ffi, test_gray) {
    qr_result result;
    zassert_equal(qr_decode_gray(&config, pixels, sizeof(pixels), 116, 116, 116, detector_scratch,
                                 sizeof(detector_scratch), core_scratch, sizeof(core_scratch),
                                 payload, sizeof(payload), segments, ARRAY_SIZE(segments), &result),
                  QR_OK);
    check_payload(&result);
    zassert_true(result.candidates > 0);
    zassert_true(result.retries > 0);
}

ZTEST(ffi, test_pixel_budget) {
    qr_result result;
    unsigned char saved[sizeof(result)];
    memset(&result, 0xa5, sizeof(result));
    memcpy(saved, &result, sizeof(result));
    zassert_equal(qr_decode_gray_with_budget(
                      &config, 1, pixels, sizeof(pixels), 116, 116, 116, detector_scratch,
                      sizeof(detector_scratch), core_scratch, sizeof(core_scratch), payload,
                      sizeof(payload), segments, ARRAY_SIZE(segments), &result),
                  QR_PIXEL_BUDGET);
    zassert_mem_equal(&result, saved, sizeof(result));
}

ZTEST_SUITE(ffi, NULL, setup, NULL, NULL, NULL);
