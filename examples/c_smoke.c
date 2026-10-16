#include "tessipher.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdalign.h>
static uint8_t core_scratch[8192], detector_scratch[8192], payload[7089];
static qr_segment segments[16];
static uint8_t matrix[56], pixels[116 * 116];
_Static_assert(sizeof(qr_config) == 5 * sizeof(uint32_t), "ABI-1 config must remain 20 bytes");
#ifndef QR_TEST_LEGACY_HEADER
static void check_mode_scratch(const qr_config *settings, const qr_memory_info *memory) {
    size_t needed = SIZE_MAX;
    const uint32_t modes[] = {QR_MODE_DIRECT, QR_MODE_FAST};
    assert(QR_DEFAULT_DETECTION_MODE == QR_MODE_DIRECT);
    assert(QR_MODE_DIRECT == 0 && QR_MODE_FAST == 2);
    for (size_t index = 0; index < sizeof modes / sizeof modes[0]; ++index) {
        uint32_t mode = modes[index];
        assert(qr_detector_scratch_bytes(settings, mode, 116, 116, &needed) == QR_OK);
        assert(needed ==
               memory->detector_scratch_bytes + (mode == QR_MODE_FAST ? (116 * 116 + 3) / 4 : 0));
        needed = SIZE_MAX;
        assert(qr_detector_scratch_bytes(settings, mode, 0, 116, &needed) == QR_INVALID_CONFIG);
        assert(needed == SIZE_MAX);
        assert(qr_detector_scratch_bytes(settings, mode, 116, memory->max_height + 1, &needed) ==
               QR_INVALID_CONFIG);
        assert(needed == SIZE_MAX);
    }
    assert(qr_detector_scratch_bytes(settings, 1, 116, 116, &needed) == QR_INVALID_CONFIG);
    assert(needed == SIZE_MAX);
    assert(qr_detector_scratch_bytes(settings, UINT32_MAX, 116, 116, &needed) == QR_INVALID_CONFIG);
    assert(needed == SIZE_MAX);
    assert(qr_detector_scratch_bytes(NULL, QR_MODE_FAST, 116, 116, &needed) == QR_INVALID_ARGUMENT);
    assert(needed == SIZE_MAX);
    assert(qr_detector_scratch_bytes(settings, QR_MODE_FAST, 116, 116, NULL) ==
           QR_INVALID_ARGUMENT);
    alignas(size_t) unsigned char misaligned[sizeof(size_t) + 1];
    memset(misaligned, 0xa5, sizeof misaligned);
    assert(qr_detector_scratch_bytes(settings, QR_MODE_FAST, 116, 116,
                                     (size_t *)(misaligned + 1)) == QR_INVALID_ARGUMENT);
    for (size_t i = 0; i < sizeof misaligned; ++i)
        assert(misaligned[i] == 0xa5);
    assert(qr_detector_scratch_bytes(settings, QR_MODE_FAST, SIZE_MAX, 116, &needed) ==
           QR_INVALID_CONFIG);
    assert(needed == SIZE_MAX);
    qr_config invalid = *settings;
    invalid.row_step = 0;
    assert(qr_detector_scratch_bytes(&invalid, QR_MODE_FAST, 116, 116, &needed) ==
           QR_INVALID_CONFIG);
    assert(needed == SIZE_MAX);
    union {
        qr_config settings;
        size_t output;
    } overlap = {0};
    overlap.settings = *settings;
    unsigned char saved[sizeof overlap];
    memcpy(saved, &overlap, sizeof overlap);
    assert(qr_detector_scratch_bytes(&overlap.settings, QR_MODE_FAST, 116, 116, &overlap.output) ==
           QR_OVERLAP);
    assert(memcmp(saved, &overlap, sizeof overlap) == 0);

    /* Exercise optional storage on an image that actually needs local thresholds. */
    static uint8_t lighting[360 * 360], expanded[50000];
    FILE *file = fopen("fixtures/lighting-v2.pgm", "rb");
    assert(file);
    unsigned width, height, maximum;
    assert(fscanf(file, "P5 %u %u %u", &width, &height, &maximum) == 3);
    assert(width == 360 && height == 360 && maximum == 255 && fgetc(file) == '\n');
    assert(fread(lighting, 1, sizeof lighting, file) == sizeof lighting);
    fclose(file);
    assert(qr_detector_scratch_bytes(settings, QR_MODE_FAST, width, height, &needed) == QR_OK);
    assert(needed <= sizeof expanded);
    qr_result result;
    qr_stats baseline, cached;
    assert(qr_decode_gray_with_mode(settings, SIZE_MAX, QR_MODE_FAST, QR_SAMPLING_CENTER, lighting,
                                    sizeof lighting, width, height, width, expanded,
                                    memory->detector_scratch_bytes, core_scratch,
                                    sizeof core_scratch, payload, sizeof payload, segments, 16,
                                    &result, &baseline) == QR_OK);
    assert(result.payload_len == 4 && memcmp(payload, "TEST", 4) == 0);
    memset(expanded + needed, 0xa5, sizeof expanded - needed);
    assert(qr_decode_gray_with_mode(settings, SIZE_MAX, QR_MODE_FAST, QR_SAMPLING_CENTER, lighting,
                                    sizeof lighting, width, height, width, expanded, needed,
                                    core_scratch, sizeof core_scratch, payload, sizeof payload,
                                    segments, 16, &result, &cached) == QR_OK);
    assert(result.payload_len == 4 && memcmp(payload, "TEST", 4) == 0);
    /* A refined global threshold can succeed before the local memo is used. */
    assert(cached.pixel_reads <= baseline.pixel_reads);
    for (size_t i = needed; i < sizeof expanded; ++i)
        assert(expanded[i] == 0xa5);
}
#endif
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
#ifndef QR_TEST_LEGACY_HEADER
    check_mode_scratch(&config, &memory);
#endif
    FILE *f = fopen("fixtures/v1.bin", "rb");
    assert(f);
    assert(fread(matrix, 1, sizeof matrix, f) == sizeof matrix);
    fclose(f);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, sizeof core_scratch, payload,
                            sizeof payload, segments, 16, &result) == 0);

#ifndef QR_TEST_LEGACY_HEADER
    size_t minimal_scratch = 0;
    assert(qr_matrix_scratch_bytes(1, &minimal_scratch) == QR_OK && minimal_scratch == 460);
    assert(qr_matrix_scratch_bytes(40, &minimal_scratch) == QR_OK && minimal_scratch == 7077);
    assert(qr_matrix_scratch_bytes(0, &minimal_scratch) == QR_VERSION && minimal_scratch == 7077);
    assert(qr_matrix_scratch_bytes(256, &minimal_scratch) == QR_VERSION);
    assert(qr_matrix_scratch_bytes(1, NULL) == QR_INVALID_ARGUMENT);
    assert(qr_matrix_scratch_bytes(1, &minimal_scratch) == QR_OK);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, minimal_scratch - 1, payload,
                            sizeof payload, segments, 16, &result) == QR_CORE_SCRATCH_TOO_SMALL);
    assert(qr_decode_matrix(matrix, sizeof matrix, 21, core_scratch, minimal_scratch, payload,
                            sizeof payload, segments, 16, &result) == QR_OK);

#endif
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
