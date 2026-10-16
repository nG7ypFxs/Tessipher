#include "heap_probe.h"

#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <new>

namespace {
void check(bool condition, const char *expression, int line) {
    if (!condition) {
        std::fprintf(stderr, "heap_probe_test:%d: %s\n", line, expression);
        std::abort();
    }
}
#define CHECK(expression) check((expression), #expression, __LINE__)
void expect(heap_probe::Snapshot value, size_t live, size_t peak, size_t calls, size_t total) {
    CHECK(value.live_bytes == live);
    CHECK(value.peak_bytes == peak);
    CHECK(value.allocation_calls == calls);
    CHECK(value.total_allocated_bytes == total);
}
void aligned(void *pointer, size_t alignment) {
    CHECK(pointer != nullptr);
    CHECK(reinterpret_cast<std::uintptr_t>(pointer) % alignment == 0);
}

void accounting() {
    heap_probe::begin(0);
    void *first = bench_malloc(13);
    void *second = bench_calloc(3, 7);
    aligned(first, alignof(std::max_align_t));
    CHECK(second != nullptr);
    for (size_t index = 0; index < 21; ++index)
        CHECK(static_cast<unsigned char *>(second)[index] == 0);
    expect(heap_probe::current(0), 34, 34, 2, 34);
    bench_free(first);
    expect(heap_probe::end(), 21, 34, 2, 34);
    heap_probe::begin(0);
    expect(heap_probe::current(0), 21, 21, 0, 0);
    bench_free(second);
    expect(heap_probe::end(), 0, 21, 0, 0);
    bench_free(nullptr);

    void *pointers[heap_probe::owners]{};
    for (size_t owner = 0; owner < heap_probe::owners; ++owner) {
        heap_probe::begin(owner);
        pointers[owner] = bench_malloc(owner + 1);
        CHECK(pointers[owner] != nullptr);
        expect(heap_probe::end(), owner + 1, owner + 1, 1, owner + 1);
    }
    // Free charges the original owner even when a different owner is active.
    heap_probe::begin(0);
    for (size_t owner = 0; owner < heap_probe::owners; ++owner)
        bench_free(pointers[owner]);
    expect(heap_probe::end(), 0, 1, 0, 0);
    for (size_t owner = 0; owner < heap_probe::owners; ++owner)
        CHECK(heap_probe::current(owner).live_bytes == 0);

    heap_probe::begin(1);
    void *tracked = bench_malloc(7);
    void *excluded;
    {
        heap_probe::Pause pause;
        excluded = bench_malloc(19);
        {
            heap_probe::Pause nested;
            void *temporary = bench_malloc(23);
            bench_free(temporary);
        }
        bench_free(tracked);
    }
    void *resumed = bench_malloc(11);
    expect(heap_probe::end(), 11, 11, 2, 18);
    bench_free(excluded);
    bench_free(resumed);
    CHECK(heap_probe::current(1).live_bytes == 0);
}

void edge_cases() {
    const size_t huge = std::numeric_limits<size_t>::max();
    heap_probe::begin(2);
    void *zero = bench_malloc(0);
    CHECK(zero != nullptr);
    expect(heap_probe::current(2), 0, 0, 1, 0);
    bench_free(zero);
    zero = bench_calloc(huge, 0);
    CHECK(zero != nullptr);
    bench_free(zero);
    errno = 0;
    CHECK(bench_malloc(huge) == nullptr);
    CHECK(errno == ENOMEM);
    errno = 0;
    CHECK(bench_calloc(huge, 2) == nullptr);
    CHECK(errno == ENOMEM);
    void *pointer = bench_realloc(nullptr, 17);
    CHECK(pointer != nullptr);
    std::memset(pointer, 0x63, 17);
    errno = 0;
    CHECK(bench_realloc(pointer, huge) == nullptr);
    CHECK(errno == ENOMEM);
    expect(heap_probe::current(2), 17, 17, 3, 17);
    for (size_t index = 0; index < 17; ++index)
        CHECK(static_cast<unsigned char *>(pointer)[index] == 0x63);
    pointer = bench_realloc(pointer, 41);
    CHECK(pointer != nullptr);
    for (size_t index = 0; index < 17; ++index)
        CHECK(static_cast<unsigned char *>(pointer)[index] == 0x63);
    expect(heap_probe::current(2), 41, 41, 4, 58);
    pointer = bench_realloc(pointer, 5);
    CHECK(pointer != nullptr);
    for (size_t index = 0; index < 5; ++index)
        CHECK(static_cast<unsigned char *>(pointer)[index] == 0x63);
    expect(heap_probe::current(2), 5, 41, 5, 63);
    CHECK(bench_realloc(pointer, 0) == nullptr);
    expect(heap_probe::end(), 0, 41, 5, 63);

    heap_probe::begin(2);
    void *sentinel = reinterpret_cast<void *>(1);
    errno = EDOM;
    CHECK(bench_posix_memalign(&sentinel, 3, 19) == EINVAL);
    CHECK(sentinel == reinterpret_cast<void *>(1));
    CHECK(errno == EDOM);
    CHECK(bench_posix_memalign(&sentinel, sizeof(void *) / 2, 19) == EINVAL);
    CHECK(bench_posix_memalign(&sentinel, 64, huge) == ENOMEM);
    CHECK(sentinel == reinterpret_cast<void *>(1));
    CHECK(errno == EDOM);
    CHECK(bench_posix_memalign(&sentinel, 256, 17) == 0);
    aligned(sentinel, 256);
    std::memset(sentinel, 0x29, 17);
    sentinel = bench_realloc(sentinel, 71);
    aligned(sentinel, 256);
    for (size_t index = 0; index < 17; ++index)
        CHECK(static_cast<unsigned char *>(sentinel)[index] == 0x29);
    sentinel = bench_realloc(sentinel, 3);
    aligned(sentinel, 256);
    CHECK(static_cast<unsigned char *>(sentinel)[2] == 0x29);
    bench_free(sentinel);
    errno = 0;
    CHECK(bench_aligned_alloc(64, 65) == nullptr);
    CHECK(errno == EINVAL);
    CHECK(bench_aligned_alloc(3, 6) == nullptr);
    void *allocation = bench_aligned_alloc(64, 128);
    aligned(allocation, 64);
    bench_free(allocation);
    expect(heap_probe::end(), 0, 128, 4, 219);

    // Successful realloc adopts the current interval owner.
    heap_probe::begin(0);
    pointer = bench_malloc(10);
    heap_probe::end();
    heap_probe::begin(3);
    pointer = bench_realloc(pointer, 20);
    CHECK(pointer != nullptr);
    CHECK(heap_probe::current(0).live_bytes == 0);
    expect(heap_probe::end(), 20, 20, 1, 20);
    bench_free(pointer);
}

void new_variants() {
    heap_probe::begin(3);
    void *ordinary = ::operator new(3);
    void *array = ::operator new[](5);
    void *nothrow = ::operator new(7, std::nothrow);
    void *nothrow_array = ::operator new[](11, std::nothrow);
    void *sized = ::operator new(13);
    void *sized_array = ::operator new[](17);
    void *wide = ::operator new(19, std::align_val_t(64));
    void *wide_array = ::operator new[](23, std::align_val_t(128));
    void *wide_nothrow = ::operator new(29, std::align_val_t(256), std::nothrow);
    void *wide_nothrow_array = ::operator new[](31, std::align_val_t(512), std::nothrow);
    void *wide_sized = ::operator new(37, std::align_val_t(64));
    void *wide_sized_array = ::operator new[](41, std::align_val_t(64));
    aligned(ordinary, alignof(std::max_align_t));
    CHECK(nothrow != nullptr);
    CHECK(nothrow_array != nullptr);
    aligned(wide, 64);
    aligned(wide_array, 128);
    aligned(wide_nothrow, 256);
    aligned(wide_nothrow_array, 512);
    expect(heap_probe::current(3), 236, 236, 12, 236);
    ::operator delete(ordinary);
    ::operator delete[](array);
    ::operator delete(nothrow, std::nothrow);
    ::operator delete[](nothrow_array, std::nothrow);
    ::operator delete(sized, size_t(13));
    ::operator delete[](sized_array, size_t(17));
    ::operator delete(wide, std::align_val_t(64));
    ::operator delete[](wide_array, std::align_val_t(128));
    ::operator delete(wide_nothrow, std::align_val_t(256), std::nothrow);
    ::operator delete[](wide_nothrow_array, std::align_val_t(512), std::nothrow);
    ::operator delete(wide_sized, size_t(37), std::align_val_t(64));
    ::operator delete[](wide_sized_array, size_t(41), std::align_val_t(64));
    expect(heap_probe::end(), 0, 236, 12, 236);

    const auto old_handler = std::set_new_handler(nullptr);
    heap_probe::begin(3);
    const size_t huge = std::numeric_limits<size_t>::max();
    CHECK(::operator new(huge, std::nothrow) == nullptr);
    CHECK(::operator new[](huge, std::nothrow) == nullptr);
    CHECK(::operator new(huge, std::align_val_t(64), std::nothrow) == nullptr);
    CHECK(::operator new[](huge, std::align_val_t(64), std::nothrow) == nullptr);
    bool threw = false;
    try {
        void *impossible = ::operator new(huge);
        ::operator delete(impossible);
    } catch (const std::bad_alloc &) {
        threw = true;
    }
    CHECK(threw);
    threw = false;
    volatile size_t invalid_alignment = 3;
    try {
        void *impossible = ::operator new(4, std::align_val_t(invalid_alignment));
        ::operator delete(impossible);
    } catch (const std::bad_alloc &) {
        threw = true;
    }
    CHECK(threw);
    expect(heap_probe::end(), 0, 0, 0, 0);
    std::set_new_handler(old_handler);
}
} // namespace

int main() {
    accounting();
    edge_cases();
    new_variants();
    for (size_t owner = 0; owner < heap_probe::owners; ++owner)
        CHECK(heap_probe::current(owner).live_bytes == 0);
    std::puts("heap_probe_test: all checks passed");
}
