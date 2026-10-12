#define _POSIX_C_SOURCE 200809L
#include "tessipher.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef struct {
    uint8_t *ds, *cs, *output;
    qr_segment *segments;
    size_t ds_len, cs_len, out_len, seg_len;
    qr_result result;
    qr_stats stats;
} buffers;
typedef struct {
    const char *name;
    uint8_t *pixels;
    size_t w, h;
} frame;
static qr_config config;
static qr_memory_info memory;
static uint8_t v1[56], v7[254];
static uint64_t now(void) {
    struct timespec t;
    assert(clock_gettime(CLOCK_MONOTONIC, &t) == 0);
    return (uint64_t)t.tv_sec * 1000000000 + (uint64_t)t.tv_nsec;
}
static int function(size_t side, size_t x, size_t y) {
    if ((x < 9 && y < 9) || (x < 9 && y >= side - 8) || (y < 9 && x >= side - 8) || x == 6 ||
        y == 6)
        return 1;
    if (side == 45) {
        if ((y < 6 && x >= side - 11) || (x < 6 && y >= side - 11))
            return 1;
        int i = -1, j = -1;
        const int centers[] = {6, 22, 38};
        for (int n = 0; n < 3; n++) {
            if (abs((int)x - centers[n]) <= 2)
                i = n;
            if (abs((int)y - centers[n]) <= 2)
                j = n;
        }
        if (i >= 0 && j >= 0)
            return !((i == 0 && (j == 0 || j == 2)) || (j == 0 && i == 2));
    }
    return 0;
}
static frame render(const char *name, size_t w, size_t h, size_t version, size_t scale, size_t ox,
                    size_t oy, int damage) {
    frame f = {name, malloc(w * h), w, h};
    assert(f.pixels);
    memset(f.pixels, 255, w * h);
    size_t side = 17 + 4 * version;
    const uint8_t *bits = version == 1 ? v1 : v7;
    uint32_t rng = 0x12345678;
    for (size_t y = 0; y < side; y++)
        for (size_t x = 0; x < side; x++) {
            int dark = (bits[(y * side + x) / 8] & (128 >> (y * side + x) % 8)) != 0;
            if (damage == 2 && !function(side, x, y)) {
                rng = rng * 1664525 + 1013904223;
                dark = (rng & 0x80000000) != 0;
            }
            for (size_t j = 0; j < scale; j++)
                for (size_t i = 0; i < scale; i++)
                    f.pixels[(oy + y * scale + j) * w + ox + x * scale + i] = dark ? 0 : 255;
            if (damage == 1 && !function(side, x, y))
                for (size_t j = scale / 2 - 1; j <= scale / 2; j++)
                    for (size_t i = scale / 2 - 1; i <= scale / 2; i++)
                        f.pixels[(oy + y * scale + j) * w + ox + x * scale + i] = dark ? 255 : 0;
        }
    return f;
}
static int compare(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}
static double median(double *a) {
    double b[15];
    memcpy(b, a, sizeof b);
    qsort(b, 15, sizeof(double), compare);
    return b[7];
}
static void distribution(double *a, double *min, double *max) {
    *min = *max = a[0];
    for (int i = 1; i < 15; i++) {
        if (a[i] < *min)
            *min = a[i];
        if (a[i] > *max)
            *max = a[i];
    }
}
static double ratio(double *a, double *b) {
    double r[15];
    for (int i = 0; i < 15; i++)
        r[i] = b[i] / fmax(a[i], 0.001);
    return median(r);
}
static int run(frame f, buffers *b, uint32_t strategy, qr_tracker *tracker, double *us) {
    size_t repetitions = (f.w == 1 && f.h == 1) ? 128 : 1;
    uint64_t start = now();
    int status = QR_INVALID_ARGUMENT;
    for (size_t repetition = 0; repetition < repetitions; repetition++) {
        if (tracker)
            status =
                qr_tracker_decode_gray(tracker, &config, 64000000, strategy, f.pixels, f.w * f.h,
                                       f.w, f.h, f.w, b->ds, b->ds_len, b->cs, b->cs_len, b->output,
                                       b->out_len, b->segments, b->seg_len, &b->result, &b->stats);
        else
            status = qr_decode_gray_with_sampling(&config, 64000000, strategy, f.pixels, f.w * f.h,
                                                  f.w, f.h, f.w, b->ds, b->ds_len, b->cs, b->cs_len,
                                                  b->output, b->out_len, b->segments, b->seg_len,
                                                  &b->result, &b->stats);
    }
    *us = (double)(now() - start) / 1000.0 / (double)repetitions;
    if (status == 0)
        assert(b->result.payload_len == 4 && memcmp(b->output, "TEST", 4) == 0);
    return status;
}
static void cases(frame *frames, size_t count, buffers *b, const char *capacity, int *gate) {
    for (size_t n = 0; n < count; n++) {
        double t[2][15];
        int outcomes[2][15];
        qr_stats work[2];
        for (int w = 0; w < 3; w++)
            for (int s = 0; s < 2; s++) {
                double unused;
                run(frames[n], &b[s], (uint32_t)s, NULL, &unused);
            }
        for (int i = 0; i < 15; i++)
            for (int order = 0; order < 2; order++) {
                int s = (i + order) % 2;
                outcomes[s][i] = run(frames[n], &b[s], (uint32_t)s, NULL, &t[s][i]);
                if (i)
                    assert(memcmp(&work[s], &b[s].stats, sizeof(qr_stats)) == 0);
                work[s] = b[s].stats;
            }
        if (outcomes[0][14] == 0 && outcomes[1][14] == 0) {
            assert(b[0].result.version == b[1].result.version &&
                   b[0].result.ec_level == b[1].result.ec_level &&
                   b[0].result.mask == b[1].result.mask);
            assert(b[0].result.payload_len == b[1].result.payload_len &&
                   memcmp(b[0].output, b[1].output, b[0].result.payload_len) == 0);
        }
        double paired = ratio(t[0], t[1]);
        int success = outcomes[0][0] != 0 || outcomes[1][0] == 0;
        int samework = outcomes[0][0] != 0 || work[0].retries != 1 ||
                       memcmp(&work[0], &work[1], sizeof(qr_stats)) == 0;
        int normal = outcomes[0][0] != 0 || paired <= 1.10;
        int failure =
            strcmp(frames[n].name, "undecodable-v7") != 0 ||
            (paired <= 1.50 && (double)work[1].pixel_reads / (double)work[0].pixel_reads <= 1.50);
        int budget = !(outcomes[0][0] >= 103 && outcomes[0][0] <= 108 &&
                       (outcomes[1][0] == 15 || outcomes[1][0] == 16 || outcomes[1][0] == 18));
        *gate &= success && samework && normal && failure && budget;
        printf("capacity=%s case=%s paired_ratio=%.4f gate=%d,%d,%d,%d,%d\n", capacity,
               frames[n].name, paired, success, samework, normal, failure, budget);
        for (int s = 0; s < 2; s++) {
            double min, max;
            distribution(t[s], &min, &max);
            printf(" strategy=%d min_us=%.3f median_us=%.3f max_us=%.3f outcomes=", s, min,
                   median(t[s]), max);
            for (int i = 0; i < 15; i++)
                printf("%s%d", i ? "," : "", outcomes[s][i]);
            printf(" reads=%zu candidates=%zu retries=%zu passes=%zu\n", work[s].pixel_reads,
                   work[s].candidates, work[s].retries, work[s].passes);
        }
    }
}
static void sequence(frame *frames, size_t count, buffers *b, const char *capacity, int tracked) {
    double times[11][2][15] = {0}, totals[2][15] = {0};
    qr_stats work[11][2];
    int outcomes[11][2];
    for (int w = 0; w < 3; w++)
        for (int s = 0; s < 2; s++) {
            qr_tracker state = {0};
            for (size_t n = 0; n < count; n++) {
                double unused;
                run(frames[n], &b[s], (uint32_t)s, tracked ? &state : NULL, &unused);
            }
        }
    for (int i = 0; i < 15; i++)
        for (int order = 0; order < 2; order++) {
            int s = (i + order) % 2;
            qr_tracker state = {0};
            for (size_t n = 0; n < count; n++) {
                int outcome =
                    run(frames[n], &b[s], (uint32_t)s, tracked ? &state : NULL, &times[n][s][i]);
                if (i) {
                    assert(outcomes[n][s] == outcome);
                    assert(memcmp(&work[n][s], &b[s].stats, sizeof(qr_stats)) == 0);
                }
                outcomes[n][s] = outcome;
                totals[s][i] += times[n][s][i];
                work[n][s] = b[s].stats;
            }
        }
    printf("capacity=%s sequence tracked=%d center_total_median_us=%.3f "
           "majority_total_median_us=%.3f paired_ratio=%.4f\n",
           capacity, tracked, median(totals[0]), median(totals[1]), ratio(totals[0], totals[1]));
    for (size_t n = 0; n < count; n++) {
        double lo[2], hi[2];
        for (int s = 0; s < 2; s++)
            distribution(times[n][s], &lo[s], &hi[s]);
        printf(" frame=%s center_us=%.3f,%.3f,%.3f majority_us=%.3f,%.3f,%.3f reads=%zu,%zu "
               "candidates=%zu,%zu retries=%zu,%zu passes=%zu,%zu\n",
               frames[n].name, lo[0], median(times[n][0]), hi[0], lo[1], median(times[n][1]), hi[1],
               work[n][0].pixel_reads, work[n][1].pixel_reads, work[n][0].candidates,
               work[n][1].candidates, work[n][0].retries, work[n][1].retries, work[n][0].passes,
               work[n][1].passes);
        for (int s = 0; s < 2; s++) {
            printf("  strategy=%d repeated_outcomes=", s);
            for (int i = 0; i < 15; i++)
                printf("%s%d", i ? "," : "", outcomes[n][s]);
            printf("\n");
        }
    }
}
int main(void) {
    assert(qr_default_config(&config) == 0 && qr_memory(&config, &memory) == 0);
    FILE *f = fopen("fixtures/v1.bin", "rb");
    assert(f);
    assert(fread(v1, 1, sizeof v1, f) == sizeof v1);
    fclose(f);
    f = fopen("fixtures/v7.bin", "rb");
    assert(f);
    assert(fread(v7, 1, sizeof v7, f) == sizeof v7);
    fclose(f);
    frame frames[] = {render("normal-v1", 640, 640, 1, 8, 140, 140, 0),
                      render("normal-v7", 640, 640, 7, 8, 140, 140, 0),
                      render("undecodable-v7", 640, 640, 7, 8, 140, 140, 2),
                      render("center-damage", 240, 240, 1, 8, 32, 32, 1),
                      render("no-qr", 640, 640, 1, 8, 140, 140, 0),
                      {"initialization-floor-1pixel", malloc(1), 1, 1}};
    /* Replace two generated placeholders before timing. */
    memset(frames[4].pixels, 255, 640 * 640);
    frames[5].pixels[0] = 255;
    frame seq[] = {render("initial", 400, 300, 1, 4, 40, 60, 0),
                   render("stable", 400, 300, 1, 4, 40, 60, 0),
                   render("small-move", 400, 300, 1, 4, 44, 64, 0),
                   render("roi-outside-move", 400, 300, 1, 4, 280, 180, 0),
                   render("disappear-first", 400, 300, 1, 4, 40, 60, 0),
                   render("disappear-second", 400, 300, 1, 4, 40, 60, 0),
                   render("reappear", 400, 300, 1, 4, 40, 60, 0),
                   render("false-candidate-first", 400, 300, 1, 4, 40, 60, 2),
                   render("false-candidate-second", 400, 300, 1, 4, 40, 60, 2),
                   render("recovered", 400, 300, 1, 4, 40, 60, 0),
                   render("resize", 320, 240, 1, 4, 160, 120, 0)};
    memset(seq[4].pixels, 255, 400 * 300);
    memset(seq[5].pixels, 255, 400 * 300);
    printf("C end-to-end includes all FFI buffer initialization; warmup=3 samples=15 alternating "
           "pairs seed=0x12345678; initialization-floor averages 128 calls per sample\n");
    int gate = 1;
    for (int large = 0; large < 2; large++) {
        buffers b[2];
        for (int s = 0; s < 2; s++) {
            b[s] = (buffers){0};
            b[s].ds_len = memory.detector_scratch_bytes;
            b[s].cs_len = memory.core_scratch_bytes;
            b[s].out_len = large ? memory.max_payload_bytes : 32;
            b[s].seg_len = large ? memory.max_segments : 8;
            b[s].ds = malloc(b[s].ds_len);
            b[s].cs = malloc(b[s].cs_len);
            b[s].output = malloc(b[s].out_len);
            b[s].segments = malloc(b[s].seg_len * sizeof(qr_segment));
            assert(b[s].ds && b[s].cs && b[s].output && b[s].segments);
        }
        const char *capacity = large ? "maximum" : "small";
        printf("capacity=%s payload=%zu segments=%zu segment_bytes=%zu\n", capacity, b[0].out_len,
               b[0].seg_len, b[0].seg_len * sizeof(qr_segment));
        cases(frames, sizeof frames / sizeof frames[0], b, capacity, &gate);
        sequence(seq, sizeof seq / sizeof seq[0], b, capacity, 0);
        sequence(seq, sizeof seq / sizeof seq[0], b, capacity, 1);
        for (int s = 0; s < 2; s++) {
            free(b[s].ds);
            free(b[s].cs);
            free(b[s].output);
            free(b[s].segments);
        }
    }
    printf("C_DEFAULT_GATE_PASSED=%d\n", gate);
    for (size_t n = 0; n < sizeof frames / sizeof frames[0]; n++)
        free(frames[n].pixels);
    for (size_t n = 0; n < sizeof seq / sizeof seq[0]; n++)
        free(seq[n].pixels);
}
