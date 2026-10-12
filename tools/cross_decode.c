/* Host-only independent quirc decoder cross-check. */
#include "vendor/quirc/quirc.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>
int main(void) {
    FILE *f = fopen("fixtures/all-model2.bin", "rb");
    assert(f);
    char magic[4];
    assert(fread(magic, 1, 4, f) == 4 && memcmp(magic, "TQR1", 4) == 0);
    unsigned count = 0;
    uint8_t header[5];
    while (fread(header, 1, 5, f) == 5) {
        if (header[1] >= 4) {
            fprintf(stderr, "Invalid ECC value: %u\n", (unsigned)header[1]);
            fclose(f);
            return 1;
        }
        struct quirc_code code = {0};
        struct quirc_data data;
        unsigned size = header[3] + 256u * header[4];
        code.size = header[0] * 4 + 17;
        assert(size <= sizeof code.cell_bitmap);
        for (unsigned i = 0; i < size; i++) {
            int c = fgetc(f);
            assert(c != EOF);
            uint8_t b = 0;
            for (int bit = 0; bit < 8; bit++)
                b |= ((c >> bit) & 1) << (7 - bit);
            code.cell_bitmap[i] = b;
        }
        assert(quirc_decode(&code, &data) == QUIRC_SUCCESS);
        char expected[8];
        snprintf(expected, sizeof expected, "%02u%c%u", header[0], "LMQH"[header[1]], header[2]);
        assert(data.payload_len == 4 && memcmp(data.payload, expected, 4) == 0);
        assert(data.version == header[0] && data.mask == header[2]);
        int ecc[] = {1, 0, 3, 2};
        assert(data.ecc_level == ecc[header[1]]);
        count++;
    }
    fclose(f);
    assert(count == 1280);
    printf("quirc independently decoded %u/1280 matrices\n", count);
}
