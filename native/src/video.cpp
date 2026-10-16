#include "video.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <mutex>
#include <thread>
#include <unordered_map>
#include <utility>

namespace {

bool SameDecodeKey(const LayerSpec& a, const LayerSpec& b) {
  // Sub-millisecond changes can cross a frame boundary. Only identical source
  // times qualify as transform-only; let the decoder resolve every actual seek.
  return a.path == b.path && a.time == b.time;
}

}  // namespace

struct VideoEngine::State {
  struct Request {
    std::vector<LayerSpec> layers;
    uint64_t serial = 0;
    uint64_t generation = 0;
  };

  mutable std::mutex mutex;
  std::condition_variable changed;
  Request request;
  std::shared_ptr<const std::vector<VideoFrame>> completed;
  uint64_t completed_request_serial = 0;
  std::atomic<uint64_t> serial{0};
  std::atomic<bool> stopping{false};
  std::thread worker;
  HWND window = nullptr;
  UINT readyMessage = 0;

  void DecodeLoop() {
    const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(initialized)) {
      fprintf(stderr, "[preview] video COM initialization failed: 0x%08lx\n", initialized);
      return;
    }
    {
      std::unordered_map<std::wstring, std::unique_ptr<HwDecoder>> pool;
      HwDecoder* shared = nullptr;
      std::vector<LayerSpec> last_decode;
      uint64_t handled = 0;
      while (!stopping.load()) {
        Request current;
        {
          std::unique_lock<std::mutex> lock(mutex);
          changed.wait(lock, [&]() { return stopping.load() || request.serial != handled; });
          if (stopping.load()) break;
          current = request;
        }
        handled = current.serial;
        const auto cancelled = [&]() {
          return stopping.load() || serial.load() != current.serial;
        };

        if (current.layers.empty()) {
          pool.clear();
          shared = nullptr;
          last_decode.clear();
          {
            std::lock_guard<std::mutex> lock(mutex);
            if (!cancelled()) completed.reset();
          }
          PostMessageW(window, readyMessage, 0, 0);
          continue;
        }
        if (current.layers.size() > kMaxLayers) {
          current.layers.resize(kMaxLayers);
        }

        // Transform-only: reuse decoded textures, just republish with new transforms.
        bool decode_unchanged = last_decode.size() == current.layers.size();
        if (decode_unchanged) {
          for (size_t i = 0; i < current.layers.size(); ++i) {
            if (!SameDecodeKey(last_decode[i], current.layers[i])) {
              decode_unchanged = false;
              break;
            }
          }
        }
        if (decode_unchanged) {
          std::shared_ptr<const std::vector<VideoFrame>> previous;
          {
            std::lock_guard<std::mutex> lock(mutex);
            previous = completed;
          }
          if (previous && previous->size() == current.layers.size()) {
            auto updated = std::make_shared<std::vector<VideoFrame>>(*previous);
            for (size_t i = 0; i < updated->size(); ++i) {
              (*updated)[i].generation = current.generation;
              (*updated)[i].transform = current.layers[i].transform;
            }
            {
              std::lock_guard<std::mutex> lock(mutex);
              if (cancelled()) continue;
              completed = std::move(updated);
              completed_request_serial = current.serial;
            }
            PostMessageW(window, readyMessage, 0, 0);
            continue;
          }
        }

        // Pool keys: path, or path#n when the same file appears twice in one stack
        // so each layer keeps an independent MF surface.
        auto pool_key_for = [](const std::vector<LayerSpec>& layers, size_t index) {
          size_t dup = 0;
          for (size_t i = 0; i < index; ++i) {
            if (layers[i].path == layers[index].path) ++dup;
          }
          if (dup == 0) return layers[index].path;
          return layers[index].path + L"#" + std::to_wstring(dup);
        };
        {
          std::vector<std::wstring> keep;
          keep.reserve(current.layers.size());
          for (size_t i = 0; i < current.layers.size(); ++i) {
            keep.push_back(pool_key_for(current.layers, i));
          }
          for (auto it = pool.begin(); it != pool.end();) {
            if (std::find(keep.begin(), keep.end(), it->first) == keep.end()) {
              if (shared == it->second.get()) shared = nullptr;
              it = pool.erase(it);
            } else {
              ++it;
            }
          }
          if (!shared && !pool.empty()) shared = pool.begin()->second.get();
        }

        auto stack = std::make_shared<std::vector<VideoFrame>>();
        stack->reserve(current.layers.size());
        bool ok = true;
        for (size_t index = 0; index < current.layers.size(); ++index) {
          const auto& layer = current.layers[index];
          if (cancelled()) {
            ok = false;
            break;
          }
          if (layer.path.empty()) continue;
          const std::wstring key = pool_key_for(current.layers, index);
          auto& slot = pool[key];
          if (!slot) {
            slot = std::make_unique<HwDecoder>();
            if (shared) {
              if (!slot->attachDevice(shared->device(), shared->context(), shared->dxgiManager(),
                                      shared->resetToken())) {
                fprintf(stderr, "[preview] cannot share decoder device\n");
                pool.erase(key);
                ok = false;
                break;
              }
            }
            if (!slot->open(layer.path.c_str())) {
              fprintf(stderr,
                      HwDecoder::IsImagePath(layer.path.c_str())
                          ? "[preview] cannot open image source\n"
                          : "[preview] cannot open video source\n");
              pool.erase(key);
              ok = false;
              break;
            }
            if (!shared) shared = slot.get();
          }
          if (cancelled()) {
            ok = false;
            break;
          }
          if (!slot->seek(layer.time, cancelled) || !slot->frame()) {
            if (!cancelled()) {
              fprintf(stderr,
                      slot->isStill() ? "[preview] cannot present still image\n"
                                      : "[preview] cannot decode at %.9f\n",
                      layer.time);
            }
            ok = false;
            break;
          }
          VideoFrame result;
          result.serial = current.serial;
          result.generation = current.generation;
          result.target = slot->isStill() ? 0.0 : layer.time;
          result.frame = *slot->frame();
          result.texture = result.frame.texture;
          result.sample = slot->sample();
          result.device = slot->device();
          result.context = slot->context();
          result.transform = layer.transform;
          stack->push_back(std::move(result));
        }
        if (!ok || cancelled()) continue;
        {
          std::lock_guard<std::mutex> lock(mutex);
          if (cancelled()) continue;
          // Publish the decode key and its textures together. A cancelled stack
          // must never make the previous textures look reusable for another time.
          last_decode = current.layers;
          completed = std::move(stack);
          completed_request_serial = current.serial;
        }
        PostMessageW(window, readyMessage, 0, 0);
      }
      pool.clear();
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
  if (!path.empty()) {
    layers.push_back(LayerSpec{std::move(path), 0.0, {}});
  }
  setLayers(std::move(layers), generation);
}

void VideoEngine::seek(double seconds) {
  if (!std::isfinite(seconds)) return;
  {
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (state_->stopping.load()) return;
    // Multi-layer stacks carry per-layer media times via setLayers.
    if (state_->request.layers.size() != 1) return;
    state_->request.layers[0].time = (std::max)(0.0, seconds);
    state_->request.serial = ++state_->serial;
  }
  state_->changed.notify_one();
}

void VideoEngine::setLayers(std::vector<LayerSpec> layers, uint64_t generation) {
  if (layers.size() > kMaxLayers) layers.resize(kMaxLayers);
  {
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (state_->stopping.load()) return;
    state_->request.layers = std::move(layers);
    state_->request.generation = generation;
    state_->request.serial = ++state_->serial;
    // Keep the last decoded stack available to the worker's transform-only
    // path. latest() still excludes it until it matches the new request.
    if (state_->request.layers.empty()) state_->completed.reset();
  }
  state_->changed.notify_one();
}

std::shared_ptr<const std::vector<VideoFrame>> VideoEngine::latest() const {
  std::lock_guard<std::mutex> lock(state_->mutex);
  if (!state_->completed || state_->completed->empty()) return {};
  if (state_->completed_request_serial != state_->serial.load()) return {};
  return state_->completed;
}

void VideoEngine::shutdown() {
  state_->stopping.store(true);
  {
    std::lock_guard<std::mutex> lock(state_->mutex);
    state_->completed.reset();
  }
  state_->changed.notify_one();
  if (state_->worker.joinable()) state_->worker.join();
}
