#pragma once

#include <atomic>
#include <cstdint>

// Shared by the stream decoders. Reserve before allocating a history texture so
// concurrent caches cannot each consume the entire preview budget.
class DecoderCacheBudget {
 public:
  explicit DecoderCacheBudget(uint64_t capacity) : capacity_(capacity) {}
  bool tryAcquire(uint64_t bytes) {
    uint64_t used = used_.load(std::memory_order_relaxed);
    do {
      if (bytes > capacity_ - used) return false;
    } while (!used_.compare_exchange_weak(used, used + bytes, std::memory_order_relaxed));
    return true;
  }
  void release(uint64_t bytes) { used_.fetch_sub(bytes, std::memory_order_relaxed); }

 private:
  const uint64_t capacity_;
  std::atomic<uint64_t> used_{0};
};
