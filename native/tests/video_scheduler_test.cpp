#include "video.h"
#include "decoder_cache_budget.h"

#include <chrono>
#include <cstdio>
#include <thread>

namespace {
int failures = 0;

void Check(bool condition, const char* message) {
  if (!condition) {
    std::fprintf(stderr, "FAIL: %s\n", message);
    ++failures;
  }
}

std::shared_ptr<const std::vector<VideoFrame>> WaitExact(VideoEngine& engine,
                                                       uint64_t generation,
                                                       size_t count) {
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
  while (std::chrono::steady_clock::now() < deadline) {
    auto stack = engine.latest();
    if (stack && stack->size() == count && stack->front().exact &&
        stack->front().generation == generation) return stack;
    std::this_thread::sleep_for(std::chrono::milliseconds(1));
  }
  Check(false, "latest coherent exact stack arrives before timeout");
  return {};
}

bool Contains(const VideoFrame& frame, double target) {
  return frame.frame.timestamp <= target + 1e-5 &&
         frame.frame.timestamp + frame.frame.duration > target - 1e-5;
}

void TestAggregateCacheReservation() {
  DecoderCacheBudget budget(100);
  Check(budget.tryAcquire(70), "first stream reserves history memory");
  Check(!budget.tryAcquire(40), "second stream cannot exceed the aggregate cache budget");
  Check(budget.tryAcquire(30), "other streams can fill the remaining shared budget");
  budget.release(70);
  Check(budget.tryAcquire(60), "evicted stream history releases capacity for another stream");
  Check(!budget.tryAcquire(11), "aggregate reservation remains bounded after eviction");
  budget.release(90);
  Check(budget.tryAcquire(100), "closing all stream caches returns the complete budget");
}

void TestColdFastLayer(const std::wstring& video, const std::wstring& still) {
  VideoEngine engine;
  engine.start(nullptr, WM_APP + 3);
  engine.setLayers({{VideoFrame::Kind::Video, video, 7.9, {}}, {VideoFrame::Kind::Video, still, 0, {0.75, 0, 1}}}, 1);
  bool saw_fast = false;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
  while (std::chrono::steady_clock::now() < deadline) {
    auto stack = engine.latest();
    if (stack && stack->size() == 1 && stack->front().frame.width == 16) {
      saw_fast = true;
      Check(!stack->front().exact, "a cold incomplete stack is approximate");
      Check(stack->front().transform.x == 0.75,
            "a partial stack preserves the original layer's transform");
    }
    if (stack && stack->size() == 2 && stack->front().exact) break;
    std::this_thread::sleep_for(std::chrono::milliseconds(1));
  }
  Check(saw_fast, "ready still publishes before the slow long-GOP layer completes");
  auto final = WaitExact(engine, 1, 2);
  Check(final && Contains(final->front(), 7.9), "cold final video contains its exact target");
}

void TestIndependentSourceChanges(const std::wstring& video, const std::wstring& still) {
  VideoEngine engine;
  engine.start(nullptr, WM_APP + 3);
  engine.setLayers({{VideoFrame::Kind::Video, still, 0, {}}, {VideoFrame::Kind::Video, still, 0, {}}}, 10);
  auto original = WaitExact(engine, 10, 2);
  if (!original) return;
  engine.setLayers({{VideoFrame::Kind::Video, still, 0, {0.25, 0, 1}}, {VideoFrame::Kind::Video, video, 7.9, {}}}, 11);
  auto retained = engine.latest();
  Check(retained && retained->size() >= 1 &&
            retained->front().texture == original->front().texture,
        "changing one source immediately retains the other stream's leased frame");
  Check(retained && !retained->front().exact && retained->front().generation == 11 &&
            retained->front().transform.x == 0.25,
        "retained stream uses the latest generation and placement, marked approximate");
  Check(!retained || retained->size() == 1 || retained->back().frame.width != 16,
        "the replaced stream's old still cannot appear in the new structure");
  auto final = WaitExact(engine, 11, 2);
  Check(final && Contains(final->back(), 7.9), "replacement finishes at the exact requested time");

  engine.setLayers({}, 12);
  Check(!engine.latest(), "empty stack invalidates all stream frames immediately");
  engine.setLayers({{VideoFrame::Kind::Video, still, 0, {}}}, 13);
  final = WaitExact(engine, 13, 1);
  Check(final && final->front().frame.width == 16,
        "source cancellation never resurrects a frame from the cleared video");
}

void TestLargeJumpsAndFinalCoherence(const std::wstring& video, const std::wstring& still) {
  VideoEngine engine;
  engine.start(nullptr, WM_APP + 3);
  engine.setLayers({{VideoFrame::Kind::Video, video, 0.1, {}}, {VideoFrame::Kind::Video, video, 0.2, {}}}, 20);
  auto initial = WaitExact(engine, 20, 2);
  if (!initial) return;
  const double initial_stamp = initial->front().frame.timestamp;
  uint64_t generation = 21;
  for (double target : {7.9, 0.15, 6.8, 1.2, 8.5, 2.35}) {
    engine.setLayers({{VideoFrame::Kind::Video, video, target, {}}, {VideoFrame::Kind::Video, video, target + 0.1, {0.1, 0, 1}}}, generation++);
    std::this_thread::sleep_for(std::chrono::milliseconds(8));
  }
  auto final = WaitExact(engine, generation - 1, 2);
  Check(final && Contains(final->front(), 2.35) && Contains(final->back(), 2.45),
        "large jumps converge to a coherent exact target in both independent streams");
  Check(initial->front().frame.timestamp == initial_stamp && initial->front().generation == 20,
        "a handed-off frame lease stays immutable while both decoders advance");

  engine.setLayers({{VideoFrame::Kind::Video, video, 7.95, {}}, {VideoFrame::Kind::Video, video, 7.85, {}}}, generation++);
  engine.setLayers({{VideoFrame::Kind::Video, still, 0, {}}, {VideoFrame::Kind::Video, still, 0, {}}}, generation++);
  final = WaitExact(engine, generation - 1, 2);
  Check(final && final->front().frame.width == 16 && final->back().frame.width == 16,
        "changing both sources cancels pending long-GOP work without stale publication");
}
}  // namespace

int wmain(int argc, wchar_t** argv) {
  if (argc != 3) return 2;
  TestAggregateCacheReservation();
  std::fprintf(stderr, "scheduler: cold fast layer\n");
  TestColdFastLayer(argv[1], argv[2]);
  std::fprintf(stderr, "scheduler: independent source changes\n");
  TestIndependentSourceChanges(argv[1], argv[2]);
  std::fprintf(stderr, "scheduler: large jumps\n");
  TestLargeJumpsAndFinalCoherence(argv[1], argv[2]);
  return failures ? 1 : 0;
}
