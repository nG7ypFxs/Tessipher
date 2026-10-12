#include "tessipher.h"
#include <assert.h>
#include <stdalign.h>
#include <stdio.h>
#include <string.h>

static _Alignas(max_align_t) uint8_t pixels[640 * 640], ds[8192], cs[8192], output[7089];
static qr_segment segments[16];
static uint8_t bits[56];
static struct {
    qr_tracker value;
    uint64_t canary;
} state;
static struct {
    qr_stats value;
    uint64_t canary[2];
} counters;
static qr_result result;
static qr_config config;
static qr_memory_info memory;

typedef struct {
    qr_tracker *tracker;
    const qr_config *config;
    size_t budget;
    uint32_t strategy;
    const uint8_t *pixels;
    size_t len, width, height, stride;
    uint8_t *ds;
    size_t ds_len;
    uint8_t *cs;
    size_t cs_len;
    uint8_t *output;
    size_t output_len;
    qr_segment *segments;
    size_t segment_len;
    qr_result *result;
    qr_stats *stats;
} args;
static args defaults(size_t w, size_t h, size_t stride) {
    args a = {&state.value,
              &config,
              64000000,
              QR_SAMPLING_CENTER_THEN_MAJORITY5,
              pixels,
              (h - 1) * stride + w,
              w,
              h,
              stride,
              ds,
              sizeof ds,
              cs,
              sizeof cs,
              output,
              32,
              segments,
              8,
              &result,
              &counters.value};
    return a;
}
static int32_t tracked(args a) {
    return qr_tracker_decode_gray(a.tracker, a.config, a.budget, a.strategy, a.pixels, a.len,
                                  a.width, a.height, a.stride, a.ds, a.ds_len, a.cs, a.cs_len,
                                  a.output, a.output_len, a.segments, a.segment_len, a.result,
                                  a.stats);
}
static int32_t stateless(args a) {
    return qr_decode_gray_with_sampling(a.config, a.budget, a.strategy, a.pixels, a.len, a.width,
                                        a.height, a.stride, a.ds, a.ds_len, a.cs, a.cs_len,
                                        a.output, a.output_len, a.segments, a.segment_len, a.result,
                                        a.stats);
}
static int function(size_t x, size_t y) {
    return (x < 9 && y < 9) || (x >= 13 && y < 9) || (x < 9 && y >= 13) || x == 6 || y == 6 ||
           x == 8 || y == 8;
}
/* damage=1 changes only data-center pixels; damage=2 randomizes complete data modules. */
static void render(args a, size_t x0, size_t y0, size_t scale, int damage) {
    memset(pixels, 255, sizeof pixels);
    uint32_t rng = 0x12345678;
    for (size_t y = 0; y < 21; y++)
        for (size_t x = 0; x < 21; x++) {
            int dark = (bits[(y * 21 + x) / 8] & (128 >> (y * 21 + x) % 8)) != 0;
            if (damage == 2 && !function(x, y)) {
                rng = rng * 1664525 + 1013904223;
                dark = (rng & 0x80000000) != 0;
            }
            for (size_t dy = 0; dy < scale; dy++)
                for (size_t dx = 0; dx < scale; dx++)
                    pixels[(y0 + y * scale + dy) * a.stride + x0 + x * scale + dx] = dark ? 0 : 255;
            if (damage == 1 && !function(x, y))
                for (size_t dy = scale / 2 - 1; dy <= scale / 2; dy++)
                    for (size_t dx = scale / 2 - 1; dx <= scale / 2; dx++)
                        pixels[(y0 + y * scale + dy) * a.stride + x0 + x * scale + dx] =
                            dark ? 255 : 0;
        }
}
static void rejection(args a, int32_t expected) {
    qr_tracker before = state.value;
    qr_result r = result;
    qr_stats s = counters.value;
    unsigned char d[32], c[32], p[32], g[sizeof segments];
    memcpy(d, ds, sizeof d);
    memcpy(c, cs, sizeof c);
    memcpy(p, output, sizeof p);
    memcpy(g, segments, sizeof g);
    assert(tracked(a) == expected);
    assert(memcmp(&before, &state.value, sizeof before) == 0);
    assert(memcmp(&r, &result, sizeof r) == 0);
    assert(memcmp(&s, &counters.value, sizeof s) == 0);
    assert(memcmp(d, ds, sizeof d) == 0);
    assert(memcmp(c, cs, sizeof c) == 0);
    assert(memcmp(p, output, sizeof p) == 0);
    assert(memcmp(g, segments, sizeof g) == 0);
    assert(state.canary == UINT64_C(0x192837465a17c0de));
    assert(counters.canary[0] == UINT64_C(0xcafebeef91234567));
}
static void terminal(args a, int32_t expected) {
    qr_tracker before = state.value;
    qr_result r = result;
    assert(tracked(a) == expected);
    assert(memcmp(&before, &state.value, sizeof before) == 0);
    assert(memcmp(&r, &result, sizeof r) == 0);
    assert(counters.value.pixel_reads <= a.budget);
}
int main(void) {
    FILE *f = fopen("fixtures/v1.bin", "rb");
    assert(f);
    assert(fread(bits, 1, sizeof bits, f) == sizeof bits);
    fclose(f);
    assert(qr_default_config(&config) == QR_OK);
    assert(qr_memory(&config, &memory) == QR_OK);
    qr_tracker_memory_info layouts;
    assert(qr_tracker_memory(&layouts) == QR_OK);
    assert(layouts.tracker_size == sizeof(qr_tracker) &&
           layouts.tracker_alignment == alignof(qr_tracker));
    assert(layouts.stats_size == sizeof(qr_stats) && layouts.stats_alignment == alignof(qr_stats));
    assert(qr_tracker_memory(NULL) == QR_INVALID_ARGUMENT);
    struct {
        qr_tracker uninitialized;
        uint64_t guard;
    } fresh;
    fresh.guard = 12345;
    assert(qr_tracker_init(&fresh.uninitialized) == QR_OK);
    assert(fresh.guard == 12345);
    qr_tracker zero = {0};
    assert(memcmp(&zero, &fresh.uninitialized, sizeof zero) == 0);
    assert(qr_tracker_init(NULL) == QR_INVALID_ARGUMENT);
    assert(qr_tracker_init((qr_tracker *)((uint8_t *)&state.value + 1)) == QR_INVALID_ARGUMENT);
    state.canary = UINT64_C(0x192837465a17c0de);
    counters.canary[0] = UINT64_C(0xcafebeef91234567);
    assert(qr_tracker_init(&state.value) == QR_OK);
    args a = defaults(400, 300, 400);
    render(a, 40, 60, 4, 0);
    assert(stateless(a) == QR_OK);
    qr_result whole = result;
    qr_stats full = counters.value;
    assert(tracked(a) == QR_OK && state.value.tag == QR_TRACKER_TRACKING);
    assert(tracked(a) == QR_OK);
    assert(counters.value.pixel_reads < full.pixel_reads / 3);
    assert(result.version == whole.version && result.payload_len == 4 &&
           memcmp(output, "TEST", 4) == 0);
    assert(result.corner0_x > 39.9f && result.corner0_x < 40.1f && result.corner0_y > 59.9f &&
           result.corner0_y < 60.1f);
    qr_tracker saved = state.value;
    render(a, 280, 180, 4, 0);
    assert(tracked(a) == QR_OK);
    size_t combined = counters.value.pixel_reads;
    state.value = saved;
    a.budget = combined - 1;
    terminal(a, QR_PIXEL_BUDGET);
    assert(counters.value.pixel_reads == a.budget);
    state.value = saved;
    a.budget = combined;
    assert(tracked(a) == QR_OK && counters.value.pixel_reads == combined);
    a.budget = 64000000;
    memset(pixels, 255, sizeof pixels);
    memset(&result, 0xa5, sizeof result);
    qr_result failed_result = result;
    assert(tracked(a) == QR_NOT_FOUND && state.value.tag == QR_TRACKER_CANDIDATE &&
           state.value.misses == 1);
    assert(memcmp(&failed_result, &result, sizeof result) == 0);
    assert(tracked(a) == QR_NOT_FOUND && state.value.tag == QR_TRACKER_SEARCHING);
    render(a, 40, 60, 4, 0);
    assert(tracked(a) == QR_OK);
    args resized = defaults(320, 240, 320);
    render(resized, 160, 120, 4, 0);
    assert(tracked(resized) == QR_OK && state.value.image_width == 320 &&
           state.value.image_height == 240);
    assert(qr_tracker_reset(&state.value) == QR_OK && state.value.tag == QR_TRACKER_SEARCHING);
    render(a, 40, 60, 4, 2);
    assert(tracked(a) == QR_REED_SOLOMON && state.value.tag == QR_TRACKER_CANDIDATE);
    assert(tracked(a) == QR_REED_SOLOMON && counters.value.passes == 8 &&
           state.value.tag == QR_TRACKER_SEARCHING);
    args padded = defaults(400, 300, 420);
    render(padded, 316, 216, 4, 0);
    assert(tracked(padded) == QR_OK);
    assert(tracked(padded) == QR_OK);
    assert(state.value.roi_x + state.value.roi_width <= 400 &&
           state.value.roi_y + state.value.roi_height <= 300);
    assert(result.corner2_x > 399.9f && result.corner2_y > 299.9f);

    args damaged = defaults(240, 240, 240);
    render(damaged, 32, 32, 8, 1);
    qr_config one = config;
    one.max_retries = 1;
    damaged.config = &one;
    damaged.strategy = QR_SAMPLING_CENTER;
    assert(stateless(damaged) == QR_RETRY_BUDGET);
    assert(counters.value.retries == 1);
    one.max_retries = 2;
    damaged.strategy = QR_SAMPLING_CENTER_THEN_MAJORITY5;
    assert(stateless(damaged) == QR_OK && counters.value.retries == 2);
    size_t exact = counters.value.pixel_reads;
    damaged.budget = exact - 1;
    assert(stateless(damaged) == QR_PIXEL_BUDGET && counters.value.pixel_reads == exact - 1);
    damaged.budget = exact;
    assert(stateless(damaged) == QR_OK);
    assert(QR_DEFAULT_SAMPLING_STRATEGY == QR_SAMPLING_CENTER_THEN_MAJORITY5);
    assert(qr_decode_gray(&config, pixels, damaged.len, 240, 240, 240, ds, sizeof ds, cs, sizeof cs,
                          output, 32, segments, 8, &result) == QR_OK &&
           result.retries == 2);
    for (uint32_t tag = 0; tag < 3; tag++) {
        state.value =
            tag == 0 ? zero : (qr_tracker){tag, tag == 1 ? 1 : 0, 19, 39, 126, 126, 400, 300};
        damaged.budget = exact - 1;
        terminal(damaged, QR_PIXEL_BUDGET);
        assert(counters.value.pixel_reads == exact - 1);
        damaged.budget = exact;
        assert(tracked(damaged) == QR_OK && counters.value.pixel_reads == exact);
    }
    a = defaults(400, 300, 400);
    render(a, 40, 60, 4, 0);
    config.max_retries = 1;
    assert(stateless(a) == QR_OK && counters.value.retries == 1);
    for (size_t y = 0; y < 300; y++)
        for (size_t x = 0; x < 200; x++) {
            uint8_t t = pixels[y * 400 + x];
            pixels[y * 400 + x] = pixels[y * 400 + 399 - x];
            pixels[y * 400 + 399 - x] = t;
        }
    assert(stateless(a) == QR_OK && counters.value.retries == 1);
    config.max_retries = 256;

    qr_tracker valid = {QR_TRACKER_TRACKING, 0, 19, 39, 126, 126, 400, 300};
    for (uint32_t tag = 0; tag < 3; tag++) {
        state.value = tag == 0 ? zero : valid;
        state.value.tag = tag;
        state.value.misses = tag == 1 ? 1 : 0;
        render(a, 40, 60, 4, 0);
        args bad = a;
        bad.tracker = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.tracker = (qr_tracker *)((uint8_t *)&state.value + 1);
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.stats = (qr_stats *)((uint8_t *)&counters.value + 1);
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.result = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.result = (qr_result *)((uint8_t *)&result + 1);
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.config = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.config = (qr_config *)((uint8_t *)&config + 1);
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.ds = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.cs = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.output = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.segments = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.segments = (qr_segment *)((uint8_t *)segments + 1);
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.strategy = 99;
        rejection(bad, QR_INVALID_CONFIG);
        bad = a;
        bad.budget = 0;
        rejection(bad, QR_INVALID_CONFIG);
        bad = a;
        bad.pixels = NULL;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.len = SIZE_MAX;
        rejection(bad, QR_INVALID_ARGUMENT);
        bad = a;
        bad.stride = 399;
        rejection(bad, QR_INVALID_IMAGE);
        /* All tracker aliases are rejected before reading integer state. */
        void *regions[] = {pixels, ds, cs, output, segments, &result, &counters.value};
        for (size_t i = 0; i < sizeof regions / sizeof regions[0]; i++) {
            bad = a;
            bad.tracker = regions[i];
            rejection(bad, QR_OVERLAP);
        }
        bad = a;
        bad.config = (qr_config *)&state.value;
        rejection(bad, QR_OVERLAP);
        /* Every stats alias, including input/config/result/state, is disjoint-checked. */
        void *stats_regions[] = {pixels, ds, cs, output, segments, &result, &state.value};
        for (size_t i = 0; i < sizeof stats_regions / sizeof stats_regions[0]; i++) {
            bad = a;
            bad.stats = stats_regions[i];
            rejection(bad, QR_OVERLAP);
        }
        bad = a;
        bad.config = (qr_config *)&counters.value;
        rejection(bad, QR_OVERLAP);
        bad = a;
        bad.ds_len = 0;
        terminal(bad, QR_DETECTOR_SCRATCH_TOO_SMALL);
        bad = a;
        bad.cs_len = 0;
        terminal(bad, QR_CORE_SCRATCH_TOO_SMALL);
        bad = a;
        bad.output_len = 0;
        terminal(bad, QR_OUTPUT_TOO_SMALL);
        bad = a;
        bad.segment_len = 0;
        terminal(bad, QR_SEGMENTS_TOO_SMALL);
        bad = a;
        bad.budget = 1;
        terminal(bad, QR_PIXEL_BUDGET);
        render(a, 40, 60, 4, 2);
        qr_config limited = config;
        limited.max_candidates = 1;
        bad = a;
        bad.config = &limited;
        terminal(bad, QR_CANDIDATE_BUDGET);
        limited = config;
        limited.max_retries = 1;
        bad = a;
        bad.config = &limited;
        terminal(bad, QR_RETRY_BUDGET);
    }
    qr_tracker invalid[15];
    for (size_t i = 0; i < 15; i++)
        invalid[i] = valid;
    invalid[0].tag = 99;
    invalid[1].misses = 1;
    invalid[2].roi_width = 0;
    invalid[3].roi_height = 0;
    invalid[4].image_width = 0;
    invalid[5].image_height = 0;
    invalid[6].roi_x = SIZE_MAX;
    invalid[7].roi_y = SIZE_MAX;
    invalid[8].roi_x = 400;
    invalid[9].roi_y = 300;
    invalid[10].image_width = (size_t)memory.max_width + 1;
    invalid[11].image_height = (size_t)memory.max_height + 1;
    invalid[12] = (qr_tracker){QR_TRACKER_CANDIDATE, 0, 19, 39, 126, 126, 400, 300};
    invalid[13] = (qr_tracker){QR_TRACKER_CANDIDATE, 2, 19, 39, 126, 126, 400, 300};
    invalid[14] = zero;
    invalid[14].roi_x = 1;
    for (size_t i = 0; i < 15; i++) {
        state.value = invalid[i];
        rejection(a, QR_INVALID_ARGUMENT);
    }
    state.value = valid;
    a.stats = NULL;
    render(a, 40, 60, 4, 0);
    assert(tracked(a) == QR_OK);
    memset(&state.value, 0xff, sizeof state.value);
    assert(qr_tracker_reset(&state.value) == QR_OK);
    assert(memcmp(&state.value, &zero, sizeof zero) == 0);
    printf("C sampling/tracker passed: tracker=%zu align=%zu stats=%zu align=%zu\n",
           layouts.tracker_size, layouts.tracker_alignment, layouts.stats_size,
           layouts.stats_alignment);
}
