/* Run from the repository root; see docs/ffi.md for build/link commands. */
#include "tessipher.h"
#include <stdio.h>
#include <stdlib.h>
#include <stdalign.h>

int main(void) {
    qr_config config;
    qr_memory_info memory;
    qr_result result;
    uint8_t matrix[56]; /* ceil(21 * 21 / 8), without a quiet zone. */
    uint8_t *core = NULL, *payload = NULL;
    qr_segment *segments = NULL;
    FILE *file = NULL;
    int exit_code = EXIT_FAILURE;
    int32_t status = qr_default_config(&config);
    if (status != QR_OK)
        goto cleanup;
    status = qr_memory(&config, &memory);
    if (status != QR_OK)
        goto cleanup;

    /* Check the linked library's layout against the header used to compile C.
       malloc meets the alignment requirements of these fundamental C types. */
    if (memory.abi_version != 1 || memory.scratch_alignment != 1 ||
        memory.segment_size != sizeof(qr_segment) ||
        memory.segment_alignment != alignof(qr_segment) ||
        memory.result_size != sizeof(qr_result) || memory.result_alignment != alignof(qr_result) ||
        memory.max_segments > SIZE_MAX / sizeof(qr_segment)) {
        fprintf(stderr, "Incompatible ABI layout\n");
        goto cleanup;
    }
    core = malloc(memory.core_scratch_bytes);
    payload = malloc(memory.max_payload_bytes);
    segments = malloc(memory.max_segments * sizeof(*segments));
    if (core == NULL || payload == NULL || segments == NULL) {
        fprintf(stderr, "Allocation failed\n");
        goto cleanup;
    }
    file = fopen("fixtures/v1.bin", "rb");
    if (file == NULL || fread(matrix, 1, sizeof(matrix), file) != sizeof(matrix)) {
        fprintf(stderr, "Cannot read fixtures/v1.bin\n");
        goto cleanup;
    }
    fclose(file);
    file = NULL;
    status = qr_decode_matrix(matrix, sizeof(matrix), 21, core, memory.core_scratch_bytes, payload,
                              memory.max_payload_bytes, segments, memory.max_segments, &result);
    if (status != QR_OK)
        goto cleanup;

    /* Payload is binary; no NUL terminator is promised. */
    printf("version=%u bytes=%zu segments=%zu\n", (unsigned)result.version, result.payload_len,
           result.segment_count);
    if (fwrite(payload, 1, result.payload_len, stdout) != result.payload_len)
        goto cleanup;
    putchar('\n');
    for (size_t i = 0; i < result.segment_count; ++i) {
        const qr_segment *s = &segments[i];
        printf("segment %zu: mode=%u offset=%zu len=%zu eci=%u\n", i, (unsigned)s->mode, s->offset,
               s->len, (unsigned)s->eci);
    }
    exit_code = EXIT_SUCCESS;
cleanup:
    if (status != QR_OK)
        fprintf(stderr, "QR status=%d\n", (int)status);
    if (file != NULL)
        fclose(file);
    free(segments);
    free(payload);
    free(core);
    return exit_code;
}
