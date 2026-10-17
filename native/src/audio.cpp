#include "audio.h"

#include <audioclient.h>
#include <avrt.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <map>
#include <mutex>
#include <thread>

using Microsoft::WRL::ComPtr;

namespace {
constexpr UINT32 kPcmRate = 48000;
constexpr double kMaxWindowSeconds = 12.5;
constexpr size_t kPcmBudgetBytes = 64 * 1024 * 1024;
constexpr double kLookAhead = 1.5;
constexpr double kTimeTolerance = 2.0 / kPcmRate;
constexpr DWORD kFirstAudio = static_cast<DWORD>(MF_SOURCE_READER_FIRST_AUDIO_STREAM);

bool ValidClip(const AudioClip& clip) {
  return !clip.path.empty() && std::isfinite(clip.start) && clip.start >= 0 &&
         std::isfinite(clip.duration) && clip.duration > 0 &&
         std::isfinite(clip.offset) && clip.offset >= 0 &&
         std::isfinite(clip.rate) && clip.rate > 0 && clip.rate <= 8 &&
         std::isfinite(clip.gain) && clip.gain >= 0 && clip.gain <= 4 &&
         std::isfinite(clip.fade_in) && std::isfinite(clip.fade_out) &&
         clip.fade_in >= 0 && clip.fade_out >= 0 &&
         clip.fade_in + clip.fade_out <= clip.duration &&
         std::isfinite(clip.start + clip.duration) &&
         std::isfinite(clip.offset + clip.duration * clip.rate);
}

double WindowEnd(const PcmWindow& window) {
  if (!window.channels || !window.sample_rate) return window.start;
  return window.start + static_cast<double>(window.samples.size() / window.channels) /
                            window.sample_rate;
}

bool Covers(const std::shared_ptr<const PcmWindow>& window, double first, double last) {
  if (!window) return false;
  if (window->failed) return true;
  return first >= window->start - kTimeTolerance &&
         (window->eof || last <= WindowEnd(*window) - 1.0 / window->sample_rate);
}

HRESULT SeekReader(IMFSourceReader* reader, double time) {
  PROPVARIANT value;
  PropVariantInit(&value);
  value.vt = VT_I8;
  value.hVal.QuadPart = static_cast<LONGLONG>(time * 10000000.0);
  const HRESULT result = reader->SetCurrentPosition(GUID_NULL, value);
  PropVariantClear(&value);
  return result;
}

bool ReadFormat(IMFSourceReader* reader, UINT32* rate, UINT32* channels) {
  ComPtr<IMFMediaType> type;
  GUID subtype{};
  UINT32 bits = 0;
  return SUCCEEDED(reader->GetCurrentMediaType(kFirstAudio,
                                               &type)) &&
         SUCCEEDED(type->GetGUID(MF_MT_SUBTYPE, &subtype)) && subtype == MFAudioFormat_Float &&
         SUCCEEDED(type->GetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, &bits)) && bits == 32 &&
         SUCCEEDED(type->GetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)) && *rate == kPcmRate &&
         SUCCEEDED(type->GetUINT32(MF_MT_AUDIO_NUM_CHANNELS, channels)) && *channels == 2;
}
}  // namespace

struct PcmDecoder::State {
  ComPtr<IMFSourceReader> reader;
  std::vector<float> pending;
  double pendingStart = 0;
  double lastRequestEnd = -1;
  double duration = 0;
  bool eof = false;
  bool mfStarted = false;
};

PcmDecoder::PcmDecoder() : state_(std::make_unique<State>()) {}
PcmDecoder::~PcmDecoder() {
  close();
  if (state_->mfStarted) MFShutdown();
}

bool PcmDecoder::open(const wchar_t* path) {
  close();
  if (!path || !*path) return false;
  if (!state_->mfStarted) {
    if (FAILED(MFStartup(MF_VERSION))) return false;
    state_->mfStarted = true;
  }
  ComPtr<IMFAttributes> attributes;
  if (FAILED(MFCreateAttributes(&attributes, 1))) return false;
  attributes->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, TRUE);
  if (FAILED(MFCreateSourceReaderFromURL(path, attributes.Get(), &state_->reader))) return false;
  auto fail = [this]() { close(); return false; };
  if (FAILED(state_->reader->SetStreamSelection(static_cast<DWORD>(MF_SOURCE_READER_ALL_STREAMS), FALSE)) ||
      FAILED(state_->reader->SetStreamSelection(kFirstAudio, TRUE))) {
    return fail();
  }
  ComPtr<IMFMediaType> type;
  if (FAILED(MFCreateMediaType(&type))) return fail();
  type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Audio);
  type->SetGUID(MF_MT_SUBTYPE, MFAudioFormat_Float);
  type->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS, 2);
  type->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND, kPcmRate);
  type->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE, 32);
  type->SetUINT32(MF_MT_AUDIO_BLOCK_ALIGNMENT, 8);
  type->SetUINT32(MF_MT_AUDIO_AVG_BYTES_PER_SECOND, kPcmRate * 8);
  type->SetUINT32(MF_MT_ALL_SAMPLES_INDEPENDENT, TRUE);
  if (FAILED(state_->reader->SetCurrentMediaType(kFirstAudio,
                                                 nullptr, type.Get()))) return fail();
  UINT32 rate = 0, channels = 0;
  if (!ReadFormat(state_->reader.Get(), &rate, &channels)) return fail();
  PROPVARIANT duration;
  PropVariantInit(&duration);
  if (SUCCEEDED(state_->reader->GetPresentationAttribute(
          static_cast<DWORD>(MF_SOURCE_READER_MEDIASOURCE), MF_PD_DURATION, &duration)) &&
      duration.vt == VT_UI8) state_->duration = duration.uhVal.QuadPart / 10000000.0;
  PropVariantClear(&duration);
  return true;
}

bool PcmDecoder::read(double start, double seconds, PcmWindow* output) {
  if (!output) return false;
  *output = PcmWindow{};
  output->start = start;
  if (!state_->reader || !std::isfinite(start) || start < 0 ||
      !std::isfinite(seconds) || seconds <= 0 || seconds > kMaxWindowSeconds ||
      !std::isfinite(start + seconds) || start > 900000000000.0) {
    output->failed = true;
    return false;
  }
  const size_t frames = static_cast<size_t>(std::ceil(seconds * kPcmRate));
  output->samples.assign(frames * 2, 0.f);
  const double end = start + static_cast<double>(frames) / kPcmRate;
  auto fail = [&]() { output->failed = true; return false; };
  // MF rejects a seek beyond the source duration with MF_E_INVALID_POSITION.
  // The requested source region is nevertheless a valid silent EOF window.
  if (state_->duration > 0 && start >= state_->duration) {
    state_->pending.clear();
    state_->eof = true;
    state_->lastRequestEnd = end;
    output->eof = true;
    return true;
  }
  // Keep the unread tail of the preceding MF sample for sequential windows.
  // Random requests are source-relative seeks; decoded PTS decide placement.
  if (state_->lastRequestEnd < 0 || start < state_->lastRequestEnd - kTimeTolerance ||
      start > state_->lastRequestEnd + 0.5) {
    state_->pending.clear();
    state_->eof = false;
    if (FAILED(SeekReader(state_->reader.Get(), start))) return fail();
  }
  state_->lastRequestEnd = end;
  unsigned emptySamples = 0;
  for (;;) {
    if (!state_->pending.empty()) {
      const double sampleEnd = state_->pendingStart +
                               state_->pending.size() / (2.0 * kPcmRate);
      if (state_->pendingStart < end && sampleEnd > start) {
        const LONGLONG destination = static_cast<LONGLONG>(
            std::llround((state_->pendingStart - start) * kPcmRate));
        const size_t sourceFirst = destination < 0 ? static_cast<size_t>(-destination) : 0;
        const size_t destinationFirst = destination > 0 ? static_cast<size_t>(destination) : 0;
        const size_t sampleFrames = state_->pending.size() / 2;
        if (sourceFirst < sampleFrames && destinationFirst < frames) {
          const size_t count = (std::min)(sampleFrames - sourceFirst, frames - destinationFirst);
          std::copy_n(state_->pending.data() + sourceFirst * 2, count * 2,
                      output->samples.data() + destinationFirst * 2);
        }
      }
      if (sampleEnd >= end || state_->pendingStart >= end) break;
      state_->pending.clear();
    }
    if (state_->eof) break;
    DWORD stream = 0, flags = 0;
    LONGLONG timestamp = 0;
    ComPtr<IMFSample> sample;
    HRESULT hr = state_->reader->ReadSample(kFirstAudio, 0,
                                            &stream, &flags, &timestamp, &sample);
    if (FAILED(hr) || (flags & MF_SOURCE_READERF_ERROR)) return fail();
    if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) {
      UINT32 rate = 0, channels = 0;
      if (!ReadFormat(state_->reader.Get(), &rate, &channels)) return fail();
    }
    if (flags & MF_SOURCE_READERF_ENDOFSTREAM) state_->eof = true;
    if (!sample) {
      if (state_->eof) break;
      if (++emptySamples > 4096) return fail();
      continue;
    }
    emptySamples = 0;
    ComPtr<IMFMediaBuffer> buffer;
    if (FAILED(sample->ConvertToContiguousBuffer(&buffer))) return fail();
    BYTE* bytes = nullptr;
    DWORD length = 0;
    if (FAILED(buffer->Lock(&bytes, nullptr, &length))) return fail();
    if (length % 8 != 0 || length > kPcmRate * 8 * 2) {
      buffer->Unlock();
      return fail();
    }
    const size_t count = length / sizeof(float);
    state_->pending.resize(count);
    if (count) std::memcpy(state_->pending.data(), bytes, length);
    buffer->Unlock();
    for (float& value : state_->pending) if (!std::isfinite(value)) value = 0.f;
    LONGLONG sampleTime = 0;
    if (SUCCEEDED(sample->GetSampleTime(&sampleTime))) timestamp = sampleTime;
    state_->pendingStart = timestamp / 10000000.0;
  }
  output->eof = state_->eof && (state_->pending.empty() ||
      state_->pendingStart + state_->pending.size() / (2.0 * kPcmRate) <= end + kTimeTolerance);
  return true;
}

void PcmDecoder::close() {
  state_->reader.Reset();
  state_->pending.clear();
  state_->lastRequestEnd = -1;
  state_->duration = 0;
  state_->eof = false;
}

void MixAudio(const std::vector<AudioClip>& clips,
              const std::vector<std::shared_ptr<const PcmWindow>>& windows,
              double timelineStart, UINT32 outputRate, UINT32 frames, float* stereo) {
  if (!stereo) return;
  std::fill_n(stereo, static_cast<size_t>(frames) * 2, 0.f);
  if (!outputRate || !std::isfinite(timelineStart)) return;
  const size_t count = (std::min)(clips.size(), windows.size());
  // Fixed stack blocks keep the render callback allocation-free. Double sums
  // preserve cancellation and clipping even for very large finite gains.
  constexpr UINT32 blockSize = 256;
  for (UINT32 block = 0; block < frames;) {
    const UINT32 blockFrames = (std::min)(blockSize, frames - block);
    double sums[blockSize * 2]{};
    for (size_t index = 0; index < count; ++index) {
      const AudioClip& clip = clips[index];
      const auto& window = windows[index];
      if (!ValidClip(clip) || !window || window->failed || !window->sample_rate ||
          !window->channels || !std::isfinite(window->start)) continue;
      const size_t sampleFrames = window->samples.size() / window->channels;
      if (!sampleFrames) continue;
      const double clipEnd = clip.start + clip.duration;
      if (clipEnd <= timelineStart || clip.start >= timelineStart +
                                                     static_cast<double>(frames) / outputRate) continue;
      for (UINT32 frame = block; frame < block + blockFrames; ++frame) {
        const double time = timelineStart + static_cast<double>(frame) / outputRate;
        if (time < clip.start || time >= clipEnd) continue;
        double sourceFrame = (clip.offset + (time - clip.start) * clip.rate - window->start) *
                             window->sample_rate;
        if (!std::isfinite(sourceFrame) || sourceFrame < -1e-6 ||
            sourceFrame >= static_cast<double>(sampleFrames)) continue;
        sourceFrame = (std::max)(0.0, sourceFrame);
        const size_t first = static_cast<size_t>(sourceFrame);
        const size_t second = (std::min)(first + 1, sampleFrames - 1);
        const double fraction = sourceFrame - first;
        for (UINT32 channel = 0; channel < 2; ++channel) {
          const UINT32 inputChannel = window->channels == 1 ? 0 : channel;
          const float left = window->samples[first * window->channels + inputChannel];
          const float right = window->samples[second * window->channels + inputChannel];
          const double elapsed = time - clip.start;
          double envelope = 1.0;
          if (clip.fade_in > 0) envelope = (std::min)(envelope, elapsed / clip.fade_in);
          if (clip.fade_out > 0) envelope = (std::min)(envelope, (clipEnd - time) / clip.fade_out);
          envelope = (std::clamp)(envelope, 0.0, 1.0);
          const double value = (left + (right - static_cast<double>(left)) * fraction) * clip.gain * envelope;
          if (std::isfinite(value)) sums[static_cast<size_t>(frame - block) * 2 + channel] += value;
        }
      }
    }
    for (size_t i = 0; i < static_cast<size_t>(blockFrames) * 2; ++i) {
      stereo[static_cast<size_t>(block) * 2 + i] = std::isfinite(sums[i]) ?
          static_cast<float>((std::clamp)(sums[i], -1.0, 1.0)) : 0.f;
    }
    block += blockFrames;
  }
}

struct AudioEngine::State {
  struct Request {
    std::shared_ptr<const std::vector<AudioClip>> clips;
    uint64_t serial = 0;
    uint64_t clipVersion = 0;
    double time = 0;
    bool play = false;
  };
  struct Decoded {
    uint64_t clipVersion = 0;
    std::vector<std::shared_ptr<const PcmWindow>> windows;
  };
  std::shared_ptr<const Request> request;
  std::shared_ptr<const Decoded> decoded;
  std::atomic<double> position{0};
  std::atomic<double> renderNext{0};
  std::atomic<uint64_t> renderSerial{0};
  std::atomic<bool> running{false};
  std::atomic<bool> deviceAvailable{false};
  std::atomic<bool> stopping{false};
  std::mutex commandMutex;
  std::thread decodeThread;
  std::thread renderThread;
  HANDLE stopEvent = nullptr;
  HANDLE decodeEvent = nullptr;
  HANDLE renderEvent = nullptr;
  bool started = false;

  State() {
    auto initial = std::make_shared<Request>();
    initial->clips = std::make_shared<const std::vector<AudioClip>>();
    request = initial;
  }

  void StartLocked() {
    if (started || stopping.load()) return;
    stopEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    decodeEvent = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    renderEvent = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    if (!stopEvent || !decodeEvent || !renderEvent) {
      stopping.store(true);
      return;
    }
    started = true;
    decodeThread = std::thread([this]() { DecodeLoop(); });
    renderThread = std::thread([this]() { RenderLoop(); });
  }

  void DecodeLoop() {
    const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(initialized)) return;
    {
      struct Reader {
        std::unique_ptr<PcmDecoder> decoder;
        bool opened = false;
        uint64_t used = 0;
      };
      std::map<std::wstring, Reader> readers;
      uint64_t iteration = 0;
      const HANDLE events[] = {stopEvent, decodeEvent};
      while (!stopping.load()) {
        if (WaitForMultipleObjects(2, events, FALSE, 20) == WAIT_OBJECT_0) break;
        const auto current = std::atomic_load(&request);
        const auto previous = std::atomic_load(&decoded);
        const double time = renderSerial.load() == current->serial ? renderNext.load() : current->time;
        auto result = std::make_shared<Decoded>();
        result->clipVersion = current->clipVersion;
        if (previous && previous->clipVersion == current->clipVersion) result->windows = previous->windows;
        else result->windows.resize(current->clips->size());
        size_t active = 0;
        for (const AudioClip& clip : *current->clips) {
          if (ValidClip(clip) && clip.start < time + kLookAhead && clip.start + clip.duration > time - 0.1) ++active;
        }
        const double maxSourceSeconds = (std::min)(kMaxWindowSeconds,
            static_cast<double>(kPcmBudgetBytes) / ((std::max)(size_t(1), active) * 8.0 * kPcmRate));
        bool changed = !previous || previous->clipVersion != current->clipVersion;
        ++iteration;
        for (size_t i = 0; i < current->clips->size() && !stopping.load(); ++i) {
          const AudioClip& clip = (*current->clips)[i];
          if (!ValidClip(clip) || clip.start >= time + kLookAhead || clip.start + clip.duration <= time - 0.1) {
            if (result->windows[i]) { result->windows[i].reset(); changed = true; }
            continue;
          }
          const double firstTime = (std::max)(time, clip.start);
          const double sourceNow = clip.offset + (firstTime - clip.start) * clip.rate;
          const double readyEnd = sourceNow + (std::min)(
              (std::min)(0.4 * clip.rate, maxSourceSeconds * 0.5),
              (clip.start + clip.duration - firstTime) * clip.rate);
          const size_t maxSamples = static_cast<size_t>(std::ceil(maxSourceSeconds * kPcmRate)) * 2;
          if (Covers(result->windows[i], sourceNow, readyEnd) &&
              result->windows[i]->samples.size() <= maxSamples) continue;
          bool shared = false;
          for (size_t j = 0; j < i; ++j) {
            if ((*current->clips)[j].path == clip.path &&
                Covers(result->windows[j], sourceNow, readyEnd) &&
                result->windows[j]->samples.size() <= maxSamples) {
              result->windows[i] = result->windows[j];
              shared = changed = true;
              break;
            }
          }
          if (shared) continue;
          const double sourceStart = (std::max)(clip.offset, sourceNow -
              (std::min)(0.1 * clip.rate, maxSourceSeconds * 0.25));
          const double seconds = (std::min)(maxSourceSeconds,
              (std::min)(kLookAhead * clip.rate + 0.2,
                         clip.offset + clip.duration * clip.rate - sourceStart + 2.0 / kPcmRate));
          if (seconds <= 0) continue;
          // Readers are reused for forward windows; retain at most 32 sources.
          auto found = readers.find(clip.path);
          if (found == readers.end()) {
            if (readers.size() >= 32) {
              auto oldest = readers.begin();
              for (auto it = readers.begin(); it != readers.end(); ++it) {
                if (it->second.used < oldest->second.used) oldest = it;
              }
              readers.erase(oldest);
            }
            Reader entry;
            entry.decoder = std::make_unique<PcmDecoder>();
            entry.opened = entry.decoder->open(clip.path.c_str());
            found = readers.emplace(clip.path, std::move(entry)).first;
          }
          found->second.used = iteration;
          auto window = std::make_shared<PcmWindow>();
          window->start = sourceStart;
          if (!found->second.opened || !found->second.decoder->read(sourceStart, seconds, window.get())) {
            window->failed = true;
          }
          if (std::atomic_load(&request)->serial != current->serial) break;
          result->windows[i] = std::move(window);
          changed = true;
        }
        if (changed && std::atomic_load(&request)->serial == current->serial) {
          std::shared_ptr<const Decoded> immutable = result;
          std::atomic_store(&decoded, std::move(immutable));
          SetEvent(renderEvent);
        }
        for (auto it = readers.begin(); it != readers.end();) {
          if (it->second.used + 100 < iteration) it = readers.erase(it);
          else ++it;
        }
      }
    }
    CoUninitialize();
  }

  bool Ready(const Request& current, const Decoded* samples, double time, UINT32 frames) const {
    const double end = time + static_cast<double>(frames) / kPcmRate;
    for (size_t i = 0; i < current.clips->size(); ++i) {
      const AudioClip& clip = (*current.clips)[i];
      if (!ValidClip(clip) || clip.start >= end || clip.start + clip.duration <= time) continue;
      if (!samples || samples->clipVersion != current.clipVersion || i >= samples->windows.size()) return false;
      const double first = clip.offset + ((std::max)(time, clip.start) - clip.start) * clip.rate;
      const double last = clip.offset + ((std::min)(end, clip.start + clip.duration) - clip.start) * clip.rate;
      if (!Covers(samples->windows[i], first, last)) return false;
    }
    return true;
  }

  void RenderLoop() {
    // Initialize the first IAudioClient on an STA thread (Windows 8 requirement).
    HRESULT initialized = E_FAIL;
    while (!stopping.load()) {
      initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
      if (SUCCEEDED(initialized)) break;
      if (WaitForSingleObject(stopEvent, 250) == WAIT_OBJECT_0) return;
    }
    if (FAILED(initialized)) return;
    // Session objects belong to one endpoint attempt. Keep the acknowledged
    // request and last confirmed consumed time across endpoint replacements.
    std::shared_ptr<const Request> current;
    double confirmedTime = 0;
    DWORD retryDelay = 250;
    while (!stopping.load()) {
      HANDLE audioEvent = CreateEventW(nullptr, FALSE, FALSE, nullptr);
      DWORD taskIndex = 0;
      HANDLE task = nullptr;
      {
        ComPtr<IMMDeviceEnumerator> enumerator;
        ComPtr<IMMDevice> device;
        ComPtr<IAudioClient> client;
        ComPtr<IAudioRenderClient> renderer;
        HRESULT hr = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                                       IID_PPV_ARGS(&enumerator));
        if (SUCCEEDED(hr)) hr = enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &device);
        if (SUCCEEDED(hr)) hr = device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr,
                                                 reinterpret_cast<void**>(client.GetAddressOf()));
        WAVEFORMATEX format{};
        format.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
        format.nChannels = 2;
        format.nSamplesPerSec = kPcmRate;
        format.wBitsPerSample = 32;
        format.nBlockAlign = 8;
        format.nAvgBytesPerSec = kPcmRate * 8;
        if (!audioEvent) hr = E_OUTOFMEMORY;
        if (SUCCEEDED(hr)) hr = client->Initialize(AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM |
            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 1000000, 0, &format, nullptr);
        if (SUCCEEDED(hr)) hr = client->SetEventHandle(audioEvent);
        UINT32 bufferFrames = 0;
        if (SUCCEEDED(hr)) hr = client->GetBufferSize(&bufferFrames);
        if (SUCCEEDED(hr)) hr = client->GetService(IID_PPV_ARGS(&renderer));
        if (SUCCEEDED(hr) && bufferFrames > 0) {
          retryDelay = 250;
          deviceAvailable.store(true);
          task = AvSetMmThreadCharacteristicsW(L"Pro Audio", &taskIndex);
          std::vector<float> mix(static_cast<size_t>(bufferFrames) * 2);
          const HANDLE events[] = {stopEvent, renderEvent, audioEvent};
          uint64_t submitted = 0;
          double base = confirmedTime;
          bool startedStream = false;
          bool freshSession = true;
          while (!stopping.load()) {
            if (WaitForMultipleObjects(3, events, FALSE, 20) == WAIT_OBJECT_0) break;
            const auto incoming = std::atomic_load(&request);
            if (freshSession || !current || incoming->serial != current->serial) {
              if (startedStream && FAILED(client->Stop())) break;
              startedStream = false;
              running.store(false);
              if (FAILED(client->Reset())) break;
              // A new request is an explicit seek/config update. Reopening the
              // same request resumes consumed output, never its original start.
              if (!current || incoming->serial != current->serial) {
                current = incoming;
                confirmedTime = current->time;
              }
              base = confirmedTime;
              freshSession = false;
              submitted = 0;
              position.store(base);
              renderNext.store(base);
              renderSerial.store(current->serial);
              SetEvent(decodeEvent);
            }
            if (!current->play) continue;
            UINT32 padding = 0;
            if (FAILED(client->GetCurrentPadding(&padding)) || padding > bufferFrames) break;
            const uint64_t consumed = submitted - (std::min)(submitted, static_cast<uint64_t>(padding));
            confirmedTime = base + static_cast<double>(consumed) / kPcmRate;
            if (std::atomic_load(&request)->serial == current->serial) position.store(confirmedTime);
            const UINT32 freeFrames = bufferFrames - padding;
            if (!freeFrames) continue;
            const double next = base + static_cast<double>(submitted) / kPcmRate;
            const auto samples = std::atomic_load(&decoded);
            if (!Ready(*current, samples.get(), next, freeFrames)) {
              // Startup and decoder starvation wait for PCM without skipping audio.
              if (startedStream) {
                if (FAILED(client->Stop())) break;
                if (FAILED(client->GetCurrentPadding(&padding))) break;
                base += static_cast<double>(submitted - (std::min)(submitted,
                            static_cast<uint64_t>(padding))) / kPcmRate;
                if (FAILED(client->Reset())) break;
                submitted = 0;
                startedStream = false;
                running.store(false);
                confirmedTime = base;
                if (std::atomic_load(&request)->serial == current->serial) position.store(confirmedTime);
              }
              renderNext.store(base + static_cast<double>(submitted) / kPcmRate);
              SetEvent(decodeEvent);
              continue;
            }
            MixAudio(*current->clips, samples ? samples->windows :
                         EmptyWindows(), next, kPcmRate, freeFrames, mix.data());
            BYTE* destination = nullptr;
            if (FAILED(renderer->GetBuffer(freeFrames, &destination))) break;
            std::memcpy(destination, mix.data(), static_cast<size_t>(freeFrames) * 8);
            if (FAILED(renderer->ReleaseBuffer(freeFrames, 0))) break;
            submitted += freeFrames;
            renderNext.store(base + static_cast<double>(submitted) / kPcmRate);
            SetEvent(decodeEvent);
            if (!startedStream) {
              if (FAILED(client->Start())) break;
              startedStream = true;
              running.store(true);
            }
          }
          // Publish failure before releasing possibly invalid device objects.
          running.store(false);
          deviceAvailable.store(false);
          if (startedStream && SUCCEEDED(client->Stop())) {
            // Some endpoint failures are transient. If padding remains readable
            // after Stop, retain every frame consumed before the stream froze.
            UINT32 finalPadding = 0;
            if (SUCCEEDED(client->GetCurrentPadding(&finalPadding)) && finalPadding <= bufferFrames) {
              const uint64_t consumed = submitted - (std::min)(submitted,
                  static_cast<uint64_t>(finalPadding));
              confirmedTime = (std::max)(confirmedTime,
                  base + static_cast<double>(consumed) / kPcmRate);
              if (std::atomic_load(&request)->serial == current->serial) position.store(confirmedTime);
            }
          }
          client->Reset();
        }
        running.store(false);
        deviceAvailable.store(false);
      }
      if (task) AvRevertMmThreadCharacteristics(task);
      if (audioEvent) CloseHandle(audioEvent);
      if (stopping.load()) break;
      const auto latest = std::atomic_load(&request);
      // Queued frames on the invalid endpoint were not confirmed consumed.
      // Rewind decoding to the resume point; pending seeks retain their time.
      renderNext.store(current && latest->serial == current->serial ? confirmedTime : latest->time);
      SetEvent(decodeEvent);
      // Retry the current default endpoint with bounded backoff. Only shutdown
      // interrupts this wait, so frequent decode/config events cannot busy-loop.
      if (WaitForSingleObject(stopEvent, retryDelay) == WAIT_OBJECT_0) break;
      retryDelay = (std::min)(DWORD(1000), retryDelay * 2);
    }
    CoUninitialize();
  }

  static const std::vector<std::shared_ptr<const PcmWindow>>& EmptyWindows() {
    static const std::vector<std::shared_ptr<const PcmWindow>> empty;
    return empty;
  }
};

AudioEngine::AudioEngine() : state_(std::make_unique<State>()) {}
AudioEngine::~AudioEngine() { shutdown(); }

void AudioEngine::setClips(std::vector<AudioClip> clips) {
  clips.erase(std::remove_if(clips.begin(), clips.end(),
                             [](const AudioClip& clip) { return !ValidClip(clip); }), clips.end());
  std::lock_guard<std::mutex> guard(state_->commandMutex);
  if (state_->stopping.load()) return;
  auto next = std::make_shared<State::Request>(*std::atomic_load(&state_->request));
  next->clips = std::make_shared<const std::vector<AudioClip>>(std::move(clips));
  next->time = state_->renderSerial.load() == next->serial ? state_->position.load() : next->time;
  ++next->serial;
  ++next->clipVersion;
  std::shared_ptr<const State::Request> immutable = next;
  std::atomic_store(&state_->request, std::move(immutable));
  state_->StartLocked();
  if (state_->renderEvent) SetEvent(state_->renderEvent);
  if (state_->decodeEvent) SetEvent(state_->decodeEvent);
}

void AudioEngine::setTransport(double time, bool playing) {
  if (!std::isfinite(time)) return;
  std::lock_guard<std::mutex> guard(state_->commandMutex);
  if (state_->stopping.load()) return;
  auto next = std::make_shared<State::Request>(*std::atomic_load(&state_->request));
  next->time = (std::max)(0.0, time);
  next->play = playing;
  ++next->serial;
  state_->position.store(next->time);
  std::shared_ptr<const State::Request> immutable = next;
  std::atomic_store(&state_->request, std::move(immutable));
  state_->StartLocked();
  if (state_->renderEvent) SetEvent(state_->renderEvent);
  if (state_->decodeEvent) SetEvent(state_->decodeEvent);
}

double AudioEngine::position() const {
  const auto current = std::atomic_load(&state_->request);
  return state_->renderSerial.load() == current->serial ? state_->position.load() : current->time;
}
bool AudioEngine::playing() const { return state_->running.load(); }
bool AudioEngine::available() const { return state_->deviceAvailable.load(); }

void AudioEngine::shutdown() {
  {
    std::lock_guard<std::mutex> guard(state_->commandMutex);
    state_->stopping.store(true);
    if (state_->stopEvent) SetEvent(state_->stopEvent);
  }
  if (state_->renderThread.joinable()) state_->renderThread.join();
  if (state_->decodeThread.joinable()) state_->decodeThread.join();
  state_->running.store(false);
  state_->deviceAvailable.store(false);
  std::atomic_store(&state_->decoded, std::shared_ptr<const State::Decoded>{});
  for (HANDLE* event : {&state_->stopEvent, &state_->decodeEvent, &state_->renderEvent}) {
    if (*event) { CloseHandle(*event); *event = nullptr; }
  }
}
