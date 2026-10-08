#include "tessipher.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdalign.h>
static uint8_t core_scratch[8192], detector_scratch[8192], payload[7089];
static qr_segment segments[16];
static uint8_t matrix[56], pixels[116 * 116];
_Static_assert(sizeof(qr_config) == 5 * sizeof(uint32_t), "ABI-1 config must remain 20 bytes");
int main(void) {
    struct {
        qr_config config;
        uint32_t canary;
    } legacy = {.canary = 0x5a17c0de};
    qr_memory_info memory;
    qr_result result;
#define config legacy.config
    assert(qr_default_config(NULL) == 1);
    assert(qr_default_config(&config) == 0);
    assert(legacy.canary == 0x5a17c0de);
    assert(qr_memory(&config, &memory) == 0);
    assert(memory.abi_version == 1 && memory.max_width >= 116 && memory.max_height >= 116);
    assert(memory.core_scratch_bytes <= sizeof core_scratch);
    assert(memory.detector_scratch_bytes <= sizeof detector_scratch);
    assert(memory.segment_size == sizeof(qr_segment));
    assert(memory.segment_alignment == alignof(qr_segment));
    assert(memory.result_size == sizeof(qr_result));
    assert(memory.result_alignment == alignof(qr_result));
    FILE *f = fopen("fixtures/v1.bin", "rb");
    assert(f);
    assert(fread(matrix, 1, sizeof matrix, f) == sizeof matrix);
    fclose(f);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, sizeof core_scratch, payload,
                            sizeof payload, segments, 16, &result) == 0);
    assert(result.version == 1 && result.ec_level == 3 && result.mask == 3 &&
           result.payload_len == 4);
    assert(memcmp(payload, "TEST", 4) == 0 && segments[0].mode == 4 && segments[0].eci == 3);
    assert(qr_decode_matrix(NULL, 1, 21, core_scratch, sizeof core_scratch, payload, sizeof payload,
                            segments, 16, &result) == 1);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, 0, payload, sizeof payload,
                            segments, 16, &result) == 102);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, sizeof core_scratch, payload,
                            3, segments, 16, &result) == 109);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, sizeof core_scratch, payload,
                            sizeof payload, NULL, 0, &result) == 110);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, sizeof core_scratch, payload,
                            sizeof payload, (qr_segment *)((uint8_t *)segments + 1), 1,
                            &result) == 1);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, matrix, sizeof matrix, payload,
                            sizeof payload, segments, 16, &result) == 2);
    assert(qr_decode_matrix(matrix, SIZE_MAX, 21, core_scratch, sizeof core_scratch, payload,
                            sizeof payload, segments, 16, &result) == 1);
    memset(pixels, 255, sizeof pixels);
    for (size_t y = 0; y < 21; y++)
        for (size_t x = 0; x < 21; x++)
            if (matrix[(y * 21 + x) / 8] & (128 >> ((y * 21 + x) % 8)))
                for (size_t j = 0; j < 4; j++)
                    for (size_t i = 0; i < 4; i++)
                        pixels[((y + 4) * 4 + j) * 116 + (x + 4) * 4 + i] = 0;
    assert(qr_decode_gray(&config, pixels, sizeof pixels, 116, 116, 116, detector_scratch,
                          sizeof detector_scratch, core_scratch, sizeof core_scratch, payload,
                          sizeof payload, segments, 16, &result) == 0);
    assert(result.payload_len == 4 && memcmp(payload, "TEST", 4) == 0);
    /* New entry point keeps ABI-1 config and result layouts and old success behavior. */
    assert(qr_decode_gray_with_budget(&config, SIZE_MAX, pixels, sizeof pixels, 116, 116, 116,
                                      detector_scratch, sizeof detector_scratch, core_scratch,
                                      sizeof core_scratch, payload, sizeof payload, segments, 16,
                                      &result) == QR_OK);
    assert(result.payload_len == 4 && memcmp(payload, "TEST", 4) == 0);
    assert(legacy.canary == 0x5a17c0de);
    memset(&result, 0xa5, sizeof result);
    unsigned char saved[sizeof result];
    memcpy(saved, &result, sizeof result);
    const size_t limits[] = {1, sizeof pixels, sizeof pixels + 1};
    for (size_t i = 0; i < sizeof limits / sizeof limits[0]; i++) {
        assert(qr_decode_gray_with_budget(&config, limits[i], pixels, sizeof pixels, 116, 116, 116,
                                          detector_scratch, sizeof detector_scratch, core_scratch,
                                          sizeof core_scratch, payload, sizeof payload, segments,
                                          16, &result) == QR_PIXEL_BUDGET);
        assert(memcmp(saved, &result, sizeof result) == 0);
        assert(legacy.canary == 0x5a17c0de);
    }
    assert(qr_decode_gray_with_budget(&config, 0, pixels, sizeof pixels, 116, 116, 116,
                                      detector_scratch, sizeof detector_scratch, core_scratch,
                                      sizeof core_scratch, payload, sizeof payload, segments, 16,
                                      &result) == QR_INVALID_CONFIG);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_gray_with_budget(&config, 1, pixels, sizeof pixels, 116, 116, 115,
                                      detector_scratch, sizeof detector_scratch, core_scratch,
                                      sizeof core_scratch, payload, sizeof payload, segments, 16,
                                      &result) == QR_INVALID_IMAGE);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_gray_with_budget(
               &config, 1, pixels, sizeof pixels, 116, 116, 116, detector_scratch,
               sizeof detector_scratch, core_scratch, sizeof core_scratch, payload, sizeof payload,
               (qr_segment *)((uint8_t *)segments + 1), 1, &result) == QR_INVALID_ARGUMENT);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_gray_with_budget(&config, 1, pixels, sizeof pixels, 116, 116, 116,
                                      detector_scratch, sizeof detector_scratch, core_scratch,
                                      sizeof core_scratch, payload, sizeof payload,
                                      (qr_segment *)pixels, 1, &result) == QR_OVERLAP);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_gray(&config, pixels, sizeof pixels, 116, 116, 115, detector_scratch,
                          sizeof detector_scratch, core_scratch, sizeof core_scratch, payload,
                          sizeof payload, segments, 16, &result) == 10);
    printf("C ABI passed: core=%zu detector=%zu alignment=%zu\n", memory.core_scratch_bytes,
           memory.detector_scratch_bytes, memory.scratch_alignment);
}
