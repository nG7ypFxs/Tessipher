#include "tessipher.h"
#include <assert.h>
#include <stdalign.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef EXPECT_MATRIX
#define EXPECT_MATRIX 0
#endif
#ifndef EXPECT_DETECT
#define EXPECT_DETECT 0
#endif

#if QR_HAS_MICRO_QR != EXPECT_MATRIX
#error "generated matrix feature macro does not match the linked test archive"
#endif
#if QR_HAS_MICRO_QR_DETECT != EXPECT_DETECT
#error "generated detector feature macro does not match the linked test archive"
#endif

#if QR_HAS_MICRO_QR
static uint8_t matrix_scratch[184];
static uint8_t payload[35];
static qr_segment segments[16];
#if QR_HAS_MICRO_QR_DETECT
static uint8_t image_scratch[184];
static uint8_t detector_scratch[8192];
static uint8_t pixels[96 * 99];
#endif

typedef struct fixture {
    uint8_t version;
    uint8_t ecc;
    uint8_t mask;
    const uint8_t *expected;
    size_t expected_len;
    const uint8_t *bits;
    size_t bits_len;
} fixture;

static uint16_t little_u16(const uint8_t *p) {
    return (uint16_t)(p[0] | (uint16_t)(p[1] << 8));
}

static fixture find_version(const uint8_t *corpus, size_t len, uint8_t version) {
    assert(len >= 4 && memcmp(corpus, "TQM1", 4) == 0);
    size_t cursor = 4;
    while (cursor < len) {
        assert(len - cursor >= 8);
        fixture item = {
            .version = corpus[cursor], .ecc = corpus[cursor + 1], .mask = corpus[cursor + 2]};
        item.expected_len = little_u16(corpus + cursor + 4);
        item.bits_len = little_u16(corpus + cursor + 6);
        cursor += 8;
        assert(item.expected_len <= len - cursor);
        item.expected = corpus + cursor;
        cursor += item.expected_len;
        assert(item.bits_len <= len - cursor);
        item.bits = corpus + cursor;
        cursor += item.bits_len;
        if (item.version == version)
            return item;
    }
    assert(!"missing Micro QR fixture version");
    abort();
}

static void matrix_tests(const uint8_t *corpus, size_t corpus_len,
                         const qr_micro_memory_info *mem) {
    assert(mem->abi_version == 1);
    assert(mem->matrix_core_scratch_bytes == sizeof matrix_scratch);
    assert(mem->max_payload_bytes == sizeof payload && mem->max_segments == 16);
    assert(mem->segment_size == sizeof(qr_segment));
    assert(mem->segment_alignment == alignof(qr_segment));
    assert(mem->result_size == sizeof(qr_micro_result));
    assert(mem->result_alignment == alignof(qr_micro_result));
    assert(mem->stats_size == sizeof(qr_stats));
    assert(mem->stats_alignment == alignof(qr_stats));
    for (uint8_t version = 1; version <= 4; version++) {
        fixture item = find_version(corpus, corpus_len, version);
        qr_micro_result result;
        memset(&result, 0xa5, sizeof result);
        const size_t side = (size_t)version * 2 + 9;
        assert(qr_decode_micro_matrix(item.bits, item.bits_len, side, matrix_scratch,
                                      sizeof matrix_scratch, payload, sizeof payload, segments, 16,
                                      &result) == QR_OK);
        assert(result.version == version && result.mask == item.mask);
        assert(result.ec_level == (item.ecc == 255 ? (uint32_t)QR_MICRO_EC_NONE : item.ecc));
        assert(result.rotation == 0 && result.mirrored == 0 && result.inverted == 0);
        assert(result.local_threshold == 0 && result.candidates == 0 && result.retries == 0);
        assert(result.payload_len == item.expected_len);
        assert(memcmp(payload, item.expected, item.expected_len) == 0);
        assert(result.segment_count > 0 && result.segment_count <= 16);
    }

    fixture item = find_version(corpus, corpus_len, 4);
    qr_micro_result result;
    unsigned char saved[sizeof result];
    memset(&result, 0xa5, sizeof result);
    memcpy(saved, &result, sizeof result);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len - 1, 17, matrix_scratch,
                                  sizeof matrix_scratch, payload, sizeof payload, segments, 16,
                                  &result) == QR_INVALID_MATRIX);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, matrix_scratch,
                                  sizeof matrix_scratch - 1, payload, sizeof payload, segments, 16,
                                  &result) == QR_CORE_SCRATCH_TOO_SMALL);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, matrix_scratch,
                                  sizeof matrix_scratch, payload, 0, segments, 16,
                                  &result) == QR_OUTPUT_TOO_SMALL);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, matrix_scratch,
                                  sizeof matrix_scratch, payload, sizeof payload, NULL, 0,
                                  &result) == QR_SEGMENTS_TOO_SMALL);
    assert(memcmp(saved, &result, sizeof result) == 0);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, matrix_scratch,
                                  sizeof matrix_scratch, payload, sizeof payload, segments, 17,
                                  &result) == QR_INVALID_ARGUMENT);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, (uint8_t *)item.bits, item.bits_len,
                                  payload, sizeof payload, segments, 16, &result) == QR_OVERLAP);
    assert(qr_decode_micro_matrix(item.bits, item.bits_len, 17, matrix_scratch,
                                  sizeof matrix_scratch, payload, sizeof payload,
                                  (qr_segment *)((uint8_t *)segments + 1), 1,
                                  &result) == QR_INVALID_ARGUMENT);
}

#if QR_HAS_MICRO_QR_DETECT
static void render(const fixture *item, size_t *width, size_t *height, size_t *stride,
                   int inverted) {
    const size_t side = (size_t)item->version * 2 + 9;
    const size_t scale = 4, quiet = 2;
    *width = (side + quiet * 2) * scale;
    *height = *width;
    *stride = *width + 3;
    memset(pixels, inverted ? 0 : 255, sizeof pixels);
    for (size_t y = 0; y < side; y++)
        for (size_t x = 0; x < side; x++) {
            const size_t bit = y * side + x;
            if ((item->bits[bit / 8] & (uint8_t)(0x80 >> (bit % 8))) == 0)
                continue;
            for (size_t dy = 0; dy < scale; dy++)
                for (size_t dx = 0; dx < scale; dx++) {
                    const size_t px = (x + quiet) * scale + dx;
                    const size_t py = (y + quiet) * scale + dy;
                    pixels[py * *stride + px] = inverted ? 255 : 0;
                }
        }
}

static void image_tests(const uint8_t *corpus, size_t corpus_len, qr_config *config,
                        const qr_micro_memory_info *mem) {
    assert(mem->image_core_scratch_bytes == sizeof image_scratch);
    assert(mem->detector_scratch_bytes <= sizeof detector_scratch);
    for (uint8_t version = 1; version <= 4; version++) {
        fixture item = find_version(corpus, corpus_len, version);
        for (int inverted = 0; inverted <= 1; inverted++) {
            size_t width, height, stride;
            render(&item, &width, &height, &stride, inverted);
            qr_micro_result result;
            assert(qr_decode_micro_gray(config, pixels, (height - 1) * stride + width, width,
                                        height, stride, detector_scratch,
                                        mem->detector_scratch_bytes, image_scratch,
                                        sizeof image_scratch, payload, sizeof payload, segments, 16,
                                        &result) == QR_OK);
            assert(result.version == version && result.inverted == (uint32_t)inverted);
            assert(result.payload_len == item.expected_len);
            assert(memcmp(payload, item.expected, item.expected_len) == 0);
            assert(result.candidates > 0 && result.retries > 0);
        }
    }

    fixture item = find_version(corpus, corpus_len, 4);
    size_t width, height, stride;
    render(&item, &width, &height, &stride, 0);
    qr_micro_result result;
    qr_stats stats;
    unsigned char saved_result[sizeof result], saved_stats[sizeof stats];
    memset(&result, 0xa5, sizeof result);
    memset(&stats, 0x5a, sizeof stats);
    memcpy(saved_result, &result, sizeof result);
    memcpy(saved_stats, &stats, sizeof stats);
    assert(qr_decode_micro_gray_with_sampling(
               config, SIZE_MAX, QR_SAMPLING_CENTER, pixels, (height - 1) * stride + width, width,
               height, stride, detector_scratch, mem->detector_scratch_bytes, image_scratch,
               sizeof image_scratch, payload, sizeof payload, segments, 16, &result,
               &stats) == QR_OK);
    assert(result.version == 4 && stats.pixel_reads > 0 && stats.retries > 0);
    memset(&result, 0xa5, sizeof result);
    memset(&stats, 0x5a, sizeof stats);
    memcpy(saved_result, &result, sizeof result);
    memcpy(saved_stats, &stats, sizeof stats);
    assert(qr_decode_micro_gray_with_sampling(
               config, 1, QR_SAMPLING_CENTER, pixels, (height - 1) * stride + width, width, height,
               stride, detector_scratch, mem->detector_scratch_bytes, image_scratch,
               sizeof image_scratch, payload, sizeof payload, segments, 16, &result,
               &stats) == QR_PIXEL_BUDGET);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(stats.pixel_reads == 1);
    memset(&stats, 0x5a, sizeof stats);
    assert(qr_decode_micro_gray_with_sampling(
               config, SIZE_MAX, 99, pixels, (height - 1) * stride + width, width, height, stride,
               detector_scratch, mem->detector_scratch_bytes, image_scratch, sizeof image_scratch,
               payload, sizeof payload, segments, 16, &result, &stats) == QR_INVALID_CONFIG);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(memcmp(saved_stats, &stats, sizeof stats) == 0);
    assert(qr_decode_micro_gray_with_budget(config, SIZE_MAX, pixels, (height - 1) * stride + width,
                                            width, height, stride, detector_scratch,
                                            mem->detector_scratch_bytes - 1, image_scratch,
                                            sizeof image_scratch, payload, sizeof payload, segments,
                                            16, &result) == QR_DETECTOR_SCRATCH_TOO_SMALL);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(qr_decode_micro_gray_with_budget(config, SIZE_MAX, pixels, (height - 1) * stride + width,
                                            width, height, stride, detector_scratch,
                                            mem->detector_scratch_bytes, image_scratch,
                                            sizeof image_scratch - 1, payload, sizeof payload,
                                            segments, 16, &result) == QR_CORE_SCRATCH_TOO_SMALL);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(qr_decode_micro_gray_with_budget(
               config, SIZE_MAX, pixels, (height - 1) * stride + width, width, height, stride,
               detector_scratch, mem->detector_scratch_bytes, image_scratch, sizeof image_scratch,
               payload, 0, segments, 16, &result) == QR_OUTPUT_TOO_SMALL);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(qr_decode_micro_gray_with_budget(
               config, SIZE_MAX, pixels, (height - 1) * stride + width, width, height, stride,
               detector_scratch, mem->detector_scratch_bytes, image_scratch, sizeof image_scratch,
               payload, sizeof payload, NULL, 0, &result) == QR_SEGMENTS_TOO_SMALL);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    qr_config invalid = *config;
    invalid.max_retries = 0;
    assert(qr_decode_micro_gray_with_sampling(
               &invalid, SIZE_MAX, QR_SAMPLING_CENTER, pixels, (height - 1) * stride + width, width,
               height, stride, detector_scratch, mem->detector_scratch_bytes, image_scratch,
               sizeof image_scratch, payload, sizeof payload, segments, 16, &result,
               &stats) == QR_INVALID_CONFIG);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
    assert(memcmp(saved_stats, &stats, sizeof stats) == 0);
    assert(qr_decode_micro_gray_with_budget(
               config, SIZE_MAX, pixels, (height - 1) * stride + width, width, height, stride,
               detector_scratch, mem->detector_scratch_bytes, image_scratch, sizeof image_scratch,
               payload, sizeof payload, (qr_segment *)pixels, 1, &result) == QR_OVERLAP);
    assert(memcmp(saved_result, &result, sizeof result) == 0);
}
#endif
#endif

int main(void) {
#if QR_HAS_MICRO_QR
    qr_config config;
    qr_micro_memory_info memory;
    assert(qr_default_config(&config) == QR_OK);
    assert(qr_micro_memory(&config, &memory) == QR_OK);
    FILE *file = fopen("fixtures/micro-segno.bin", "rb");
    assert(file != NULL && fseek(file, 0, SEEK_END) == 0);
    long file_len = ftell(file);
    assert(file_len > 0 && fseek(file, 0, SEEK_SET) == 0);
    uint8_t *corpus = malloc((size_t)file_len);
    assert(corpus != NULL && fread(corpus, 1, (size_t)file_len, file) == (size_t)file_len);
    fclose(file);
    matrix_tests(corpus, (size_t)file_len, &memory);
#if QR_HAS_MICRO_QR_DETECT
    image_tests(corpus, (size_t)file_len, &config, &memory);
#else
    assert(memory.image_core_scratch_bytes == 0 && memory.detector_scratch_bytes == 0);
#endif
    free(corpus);
#endif
    puts("Micro QR C ABI passed");
    return 0;
}
