#ifndef TESSIPHER_DECODER_BENCH_HEAP_PROBE_H
#define TESSIPHER_DECODER_BENCH_HEAP_PROBE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#define BENCH_NOEXCEPT noexcept
#else
#define BENCH_NOEXCEPT
#endif

void *bench_malloc(size_t size) BENCH_NOEXCEPT;
void *bench_calloc(size_t count, size_t size) BENCH_NOEXCEPT;
void *bench_realloc(void *pointer, size_t size) BENCH_NOEXCEPT;
void bench_free(void *pointer) BENCH_NOEXCEPT;
int bench_posix_memalign(void **result, size_t alignment, size_t size) BENCH_NOEXCEPT;
void *bench_aligned_alloc(size_t alignment, size_t size) BENCH_NOEXCEPT;

#undef BENCH_NOEXCEPT
#ifdef __cplusplus
}

namespace heap_probe {
inline constexpr size_t owners = 4;
struct Snapshot {
    size_t live_bytes;
    size_t peak_bytes;
    size_t allocation_calls;
    size_t total_allocated_bytes;
};

// Single-thread benchmark instrumentation; begin resets interval counters while
// retaining allocations that are still live from an earlier interval.
void begin(size_t owner) noexcept;
Snapshot end() noexcept;
Snapshot current(size_t owner) noexcept;

class Pause {
  public:
    Pause() noexcept;
    ~Pause();
    Pause(const Pause &) = delete;
    Pause &operator=(const Pause &) = delete;

  private:
    size_t saved_owner_;
};
} // namespace heap_probe
#endif
#endif
