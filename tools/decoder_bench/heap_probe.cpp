#include "heap_probe.h"

#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <new>

namespace {
constexpr size_t no_owner = heap_probe::owners;
size_t active_owner = no_owner;
heap_probe::Snapshot snapshots[heap_probe::owners]{};

struct Header {
    void *raw;
    size_t requested;
    size_t alignment;
    size_t owner;
};

bool valid_alignment(size_t alignment) noexcept {
    return alignment != 0 && (alignment & (alignment - 1)) == 0;
}

bool allocation_size(size_t size, size_t alignment, size_t &total) noexcept {
    const size_t limit = std::numeric_limits<size_t>::max();
    if (alignment - 1 > limit - sizeof(Header) || size > limit - sizeof(Header) - (alignment - 1)) {
        errno = ENOMEM;
        return false;
    }
    total = size + sizeof(Header) + alignment - 1;
    return true;
}

void *aligned_pointer(void *raw, size_t alignment) noexcept {
    const auto address = reinterpret_cast<std::uintptr_t>(raw) + sizeof(Header);
    return reinterpret_cast<void *>((address + alignment - 1) & ~(alignment - 1));
}

Header *header(void *pointer) noexcept {
    return reinterpret_cast<Header *>(static_cast<unsigned char *>(pointer) - sizeof(Header));
}

void charge(size_t owner, size_t size) noexcept {
    if (owner == no_owner)
        return;
    auto &value = snapshots[owner];
    value.live_bytes += size;
    if (value.live_bytes > value.peak_bytes)
        value.peak_bytes = value.live_bytes;
    ++value.allocation_calls;
    value.total_allocated_bytes += size;
}

void uncharge(size_t owner, size_t size) noexcept {
    if (owner != no_owner)
        snapshots[owner].live_bytes -= size;
}

void *allocate(size_t size, size_t alignment) noexcept {
    if (!valid_alignment(alignment)) {
        errno = EINVAL;
        return nullptr;
    }
    if (alignment < alignof(std::max_align_t))
        alignment = alignof(std::max_align_t);
    size_t total;
    if (!allocation_size(size, alignment, total))
        return nullptr;
    void *raw = std::malloc(total);
    if (raw == nullptr) {
        errno = ENOMEM;
        return nullptr;
    }
    void *result = aligned_pointer(raw, alignment);
    *header(result) = {raw, size, alignment, active_owner};
    charge(active_owner, size);
    return result;
}

void *throwing_allocate(size_t size, size_t alignment) {
    for (;;) {
        if (void *result = allocate(size, alignment))
            return result;
        if (errno == EINVAL)
            throw std::bad_alloc();
        const auto handler = std::get_new_handler();
        if (handler == nullptr)
            throw std::bad_alloc();
        handler();
    }
}
} // namespace

namespace heap_probe {
void begin(size_t owner) noexcept {
    if (owner >= owners)
        std::abort();
    active_owner = owner;
    auto &value = snapshots[owner];
    value.peak_bytes = value.live_bytes;
    value.allocation_calls = 0;
    value.total_allocated_bytes = 0;
}

Snapshot end() noexcept {
    if (active_owner == no_owner)
        std::abort();
    const auto value = snapshots[active_owner];
    active_owner = no_owner;
    return value;
}

Snapshot current(size_t owner) noexcept {
    if (owner >= owners)
        std::abort();
    return snapshots[owner];
}

Pause::Pause() noexcept : saved_owner_(active_owner) {
    active_owner = no_owner;
}
Pause::~Pause() {
    active_owner = saved_owner_;
}
} // namespace heap_probe

extern "C" {
void *bench_malloc(size_t size) noexcept {
    return allocate(size, alignof(std::max_align_t));
}

void *bench_calloc(size_t count, size_t size) noexcept {
    if (size != 0 && count > std::numeric_limits<size_t>::max() / size) {
        errno = ENOMEM;
        return nullptr;
    }
    const size_t bytes = count * size;
    void *result = bench_malloc(bytes);
    if (result != nullptr)
        std::memset(result, 0, bytes);
    return result;
}

void bench_free(void *pointer) noexcept {
    if (pointer == nullptr)
        return;
    const Header value = *header(pointer);
    uncharge(value.owner, value.requested);
    std::free(value.raw);
}

void *bench_realloc(void *pointer, size_t size) noexcept {
    if (pointer == nullptr)
        return bench_malloc(size);
    if (size == 0) {
        bench_free(pointer);
        return nullptr;
    }
    const Header old = *header(pointer);
    const auto old_offset =
        static_cast<unsigned char *>(pointer) - static_cast<unsigned char *>(old.raw);
    size_t total;
    if (!allocation_size(size, old.alignment, total))
        return nullptr;
    void *raw = std::realloc(old.raw, total);
    if (raw == nullptr) {
        errno = ENOMEM;
        return nullptr;
    }
    void *result = aligned_pointer(raw, old.alignment);
    // realloc preserves the old offset, which can differ after an aligned move.
    std::memmove(result, static_cast<unsigned char *>(raw) + old_offset,
                 old.requested < size ? old.requested : size);
    *header(result) = {raw, size, old.alignment, active_owner};
    uncharge(old.owner, old.requested);
    charge(active_owner, size);
    return result;
}

int bench_posix_memalign(void **result, size_t alignment, size_t size) noexcept {
    const int saved_errno = errno;
    if (!valid_alignment(alignment) || alignment % sizeof(void *) != 0) {
        errno = saved_errno;
        return EINVAL;
    }
    void *allocation = allocate(size, alignment);
    const int status = allocation != nullptr ? 0 : ENOMEM;
    if (allocation != nullptr)
        *result = allocation;
    errno = saved_errno;
    return status;
}

void *bench_aligned_alloc(size_t alignment, size_t size) noexcept {
    if (!valid_alignment(alignment) || size % alignment != 0) {
        errno = EINVAL;
        return nullptr;
    }
    return allocate(size, alignment);
}
}

void *operator new(size_t size) {
    return throwing_allocate(size, alignof(std::max_align_t));
}
void *operator new[](size_t size) {
    return ::operator new(size);
}
void *operator new(size_t size, const std::nothrow_t &) noexcept {
    try {
        return ::operator new(size);
    } catch (...) {
        return nullptr;
    }
}
void *operator new[](size_t size, const std::nothrow_t &) noexcept {
    try {
        return ::operator new[](size);
    } catch (...) {
        return nullptr;
    }
}
void *operator new(size_t size, std::align_val_t alignment) {
    return throwing_allocate(size, static_cast<size_t>(alignment));
}
void *operator new[](size_t size, std::align_val_t alignment) {
    return ::operator new(size, alignment);
}
void *operator new(size_t size, std::align_val_t alignment, const std::nothrow_t &) noexcept {
    try {
        return ::operator new(size, alignment);
    } catch (...) {
        return nullptr;
    }
}
void *operator new[](size_t size, std::align_val_t alignment, const std::nothrow_t &) noexcept {
    try {
        return ::operator new[](size, alignment);
    } catch (...) {
        return nullptr;
    }
}
void operator delete(void *pointer) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer) noexcept {
    bench_free(pointer);
}
void operator delete(void *pointer, size_t) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer, size_t) noexcept {
    bench_free(pointer);
}
void operator delete(void *pointer, const std::nothrow_t &) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer, const std::nothrow_t &) noexcept {
    bench_free(pointer);
}
void operator delete(void *pointer, std::align_val_t) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer, std::align_val_t) noexcept {
    bench_free(pointer);
}
void operator delete(void *pointer, size_t, std::align_val_t) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer, size_t, std::align_val_t) noexcept {
    bench_free(pointer);
}
void operator delete(void *pointer, std::align_val_t, const std::nothrow_t &) noexcept {
    bench_free(pointer);
}
void operator delete[](void *pointer, std::align_val_t, const std::nothrow_t &) noexcept {
    bench_free(pointer);
}
