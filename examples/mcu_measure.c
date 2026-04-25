/* Board integration sample; board_cycles/board_record are supplied by firmware. */
#include "tessipher.h"
extern uint32_t board_cycles(void);
extern void board_record(uint32_t elapsed_cycles, int32_t status);
int32_t measured_decode(const qr_config *config, const uint8_t *pixels, size_t pixels_len,
    size_t width, size_t height, size_t stride,
    uint8_t *detector_scratch, size_t detector_len, uint8_t *core_scratch, size_t core_len,
    uint8_t *payload, size_t payload_len, qr_segment *segments, size_t segment_count,
    qr_result *result) {
    uint32_t start = board_cycles();
    int32_t status = qr_decode_gray(config, pixels, pixels_len, width, height, stride,
        detector_scratch, detector_len, core_scratch, core_len, payload, payload_len,
        segments, segment_count, result);
    uint32_t elapsed = board_cycles() - start; /* unsigned wrap, for < 2^32 cycles */
    board_record(elapsed, status);
    return status;
}
