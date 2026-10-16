#include "video.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <mutex>
#include <thread>
#include <utility>

namespace {
constexpr uint64_t kHistoryBytes = 256ull * 1024 * 1024;
}  // namespace

struct VideoEngine::State {
  struct Request {
    std::vector<LayerSpec> layers;
    uint64_t serial = 0;
    uint64_t generation = 0;
    uint64_t structure = 0;
    std::array<uint64_t, kMaxLayers> epochs{};
  };

  struct LayerOutcome {
    // Captured only by the decoder's owning worker; composition never reads a
    // decoder while that worker is advancing or replacing its MF surface.
    std::optional<VideoFrame> frame;
    double served = 0;
    bool exact = false;
    bool still = false;
  };

  mutable std::mutex mutex;
  std::condition_variable changed;
  Request request;
  std::array<LayerOutcome, kMaxLayers> outcomes;
  std::shared_ptr<const std::vector<VideoFrame>> completed;
  std::shared_ptr<DecoderCacheBudget> history = std::make_shared<DecoderCacheBudget>(kHistoryBytes);
  std::atomic<uint64_t> serial{0};
  std::array<std::atomic<uint64_t>, kMaxLayers> epochs{};
  std::atomic<bool> stopping{false};
  uint64_t publication = 0;
  std::thread worker;
  HWND window = nullptr;
  UINT readyMessage = 0;

  bool ExactLocked(size_t index) const {
    const auto& outcome = outcomes[index];
    return outcome.frame && outcome.exact &&
           (outcome.still || outcome.served == request.layers[index].time);
  }

  // With mutex held, publish every available layer in its original stacking
  // order. Missing cold layers do not block ready ones. A retained frame belongs
  // to its stream epoch, so changing another source (e.g. a text raster) neither
  // discards it nor cancels video progress.
  bool ComposeLocked(bool decoded) {
    auto stack = std::make_shared<std::vector<VideoFrame>>();
    stack->reserve(request.layers.size());
    bool exact = !request.layers.empty();
    for (size_t index = 0; index < request.layers.size(); ++index) {
      const auto& outcome = outcomes[index];
      exact = exact && ExactLocked(index);
      if (!outcome.frame) continue;
      VideoFrame frame = *outcome.frame;
      frame.target = outcome.still ? 0.0 : request.layers[index].time;
      frame.transform = request.layers[index].transform;
      frame.structure = request.structure;
      frame.generation = request.generation;
      frame.topmost = index + 1 == request.layers.size();
      stack->push_back(std::move(frame));
    }
    if (stack->empty()) {
      const bool had_frames = completed != nullptr;
      completed.reset();
      return had_frames;
    }
    if (decoded || !completed || completed->size() != stack->size() ||
        completed->front().exact != exact) ++publication;
    for (auto& frame : *stack) {
      frame.serial = publication;
      frame.exact = exact;
    }
    completed = std::move(stack);
    return true;
  }

  void Notify() const { PostMessageW(window, readyMessage, 0, 0); }

  void DecodeStream(size_t index, HwDecoder& shared) {
    const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(initialized)) {
      fprintf(stderr, "[preview] stream COM initialization failed: 0x%08lx\n", initialized);
      return;
    }
    {
      HwDecoder decoder;
      if (!decoder.attachDevice(shared.device(), shared.context(), shared.dxgiManager(),
                                shared.resetToken())) {
        fprintf(stderr, "[preview] cannot share decoder device\n");
      } else {
        std::wstring path;
        uint64_t handled = 0;
        while (!stopping.load()) {
          Request current;
          {
            std::unique_lock<std::mutex> lock(mutex);
            changed.wait(lock, [&]() { return stopping.load() || request.serial != handled; });
            if (stopping.load()) break;
            current = request;
            handled = current.serial;
            if (index < current.layers.size() && ExactLocked(index)) continue;
          }
          if (index >= current.layers.size()) {
            decoder.close();
            path.clear();
            continue;
          }
          const auto epoch = current.epochs[index];
          const auto cancelled = [&]() {
            return stopping.load() || epochs[index].load() != epoch;
          };
          const auto newer = [&]() -> std::optional<double> {
            if (serial.load() == current.serial) return std::nullopt;
            std::lock_guard<std::mutex> lock(mutex);
            if (index >= request.layers.size() || request.epochs[index] != epoch) return std::nullopt;
            return request.layers[index].time;
          };
          const auto& layer = current.layers[index];
          decoder.setCacheBudget(history, kHistoryBytes / current.layers.size());
          if (cancelled()) continue;
          if (path != layer.path) {
            decoder.close();
            path.clear();
            if (!decoder.open(layer.path.c_str())) {
              if (!cancelled()) fprintf(stderr, "[preview] cannot open stream source\n");
              continue;
            }
            path = layer.path;
          }
          if (cancelled()) continue;
          double served = layer.time;
          const SeekResult result = decoder.seek(layer.time, SeekControl{cancelled, newer}, &served);
          if (cancelled()) continue;
          if (result == SeekResult::Failed) {
            fprintf(stderr, "[preview] cannot decode at %.9f\n", served);
            continue;
          }
          if (!decoder.frame() || (result != SeekResult::Exact && result != SeekResult::Partial)) continue;
          VideoFrame captured;
          captured.runtime = decoder.runtime();
          captured.frame = *decoder.frame();
          captured.texture = captured.frame.texture;
          captured.sample = decoder.sample();
          captured.device = decoder.device();
          captured.context = decoder.context();
          bool published = false;
          {
            std::lock_guard<std::mutex> lock(mutex);
            if (cancelled() || index >= request.layers.size()) continue;
            auto& outcome = outcomes[index];
            const double want = request.layers[index].time;
            const bool exact = result == SeekResult::Exact && (decoder.isStill() || served == want);
            // Reverse preroll can be farther from the new target than the last
            // visible frame. Keep the closer lease until a better/exact one arrives.
            if (exact || !outcome.frame ||
                (!ExactLocked(index) &&
                 std::abs(captured.frame.timestamp - want) <=
                     std::abs(outcome.frame->frame.timestamp - want))) {
              const bool decoded = !outcome.frame ||
                  outcome.frame->texture != captured.texture ||
                  outcome.frame->frame.timestamp != captured.frame.timestamp;
              outcome.frame = std::move(captured);
              outcome.served = served;
              outcome.exact = result == SeekResult::Exact;
              outcome.still = decoder.isStill();
              published = ComposeLocked(decoded);
            }
            if (ExactLocked(index)) handled = request.serial;
          }
          if (published) Notify();
        }
      }
    }  // Decoder surfaces and MF readers are released on their owning COM worker.
    CoUninitialize();
  }

  void DecodeLoop() {
    const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(initialized)) {
      fprintf(stderr, "[preview] video COM initialization failed: 0x%08lx\n", initialized);
      return;
    }
    {
      HwDecoder shared;
      if (!shared.prepareDevice()) {
        fprintf(stderr, "[preview] cannot initialize decoder device\n");
      } else {
        // A fixed maximum, independent of pointer/request rate. Each worker owns
        // one MF reader, including duplicate paths at different media times.
        std::array<std::thread, kMaxLayers> streams;
        for (size_t index = 0; index < streams.size(); ++index) {
          streams[index] = std::thread([&, index]() { DecodeStream(index, shared); });
        }
        for (auto& stream : streams) stream.join();
      }
    }
    CoUninitialize();
  }
};

VideoEngine::VideoEngine() : state_(std::make_unique<State>()) {}
VideoEngine::~VideoEngine() { shutdown(); }

void VideoEngine::start(HWND window, UINT readyMessage) {
  if (state_->worker.joinable() || state_->stopping.load()) return;
  state_->window = window;
  state_->readyMessage = readyMessage;
  state_->worker = std::thread([this]() { state_->DecodeLoop(); });
}

void VideoEngine::setPath(std::wstring path, uint64_t generation) {
  std::vector<LayerSpec> layers;
  if (!path.empty()) layers.push_back(LayerSpec{std::move(path), 0.0, {}});
  setLayers(std::move(layers), generation);
}

void VideoEngine::seek(double seconds) {
  if (!std::isfinite(seconds)) return;
  bool published = false;
  {
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (state_->stopping.load() || state_->request.layers.size() != 1) return;
    state_->request.layers[0].time = (std::max)(0.0, seconds);
    state_->request.serial = ++state_->serial;
    published = state_->ComposeLocked(false);
  }
  state_->changed.notify_all();
  if (published) state_->Notify();
}

void VideoEngine::setLayers(std::vector<LayerSpec> layers, uint64_t generation) {
  if (layers.size() > kMaxLayers) layers.resize(kMaxLayers);
  bool published = false;
  {
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (state_->stopping.load()) return;
    bool structure_changed = layers.size() != state_->request.layers.size();
    for (size_t index = 0; index < kMaxLayers; ++index) {
      const bool had = index < state_->request.layers.size();
      const bool have = index < layers.size();
      if (had != have || (have && state_->request.layers[index].path != layers[index].path)) {
        state_->request.epochs[index] = ++state_->epochs[index];
        state_->outcomes[index] = {};
        structure_changed = true;
      }
    }
    if (structure_changed) ++state_->request.structure;
    state_->request.layers = std::move(layers);
    state_->request.generation = generation;
    state_->request.serial = ++state_->serial;
    published = state_->ComposeLocked(false);
  }
  state_->changed.notify_all();
  if (published) state_->Notify();
}

std::shared_ptr<const std::vector<VideoFrame>> VideoEngine::latest() const {
  std::lock_guard<std::mutex> lock(state_->mutex);
  return state_->completed;
}

void VideoEngine::shutdown() {
  state_->stopping.store(true);
  state_->changed.notify_all();
  if (state_->worker.joinable()) state_->worker.join();
  std::lock_guard<std::mutex> lock(state_->mutex);
  state_->completed.reset();
  for (auto& outcome : state_->outcomes) outcome = {};
}
