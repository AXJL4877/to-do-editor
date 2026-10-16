#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <memory>
#include <string>
#include <vector>

struct AudioClip {
  std::wstring path;
  double start = 0;
  double duration = 0;
  double offset = 0;
  double rate = 1;
  float gain = 1;
};

// Samples are interleaved PCM. start is the source timestamp of sample zero;
// zero samples within the window preserve source timestamp gaps.
struct PcmWindow {
  double start = 0;
  UINT32 sample_rate = 48000;
  UINT32 channels = 2;
  std::vector<float> samples;
  bool eof = false;
  bool failed = false;
};

// One decoder belongs to one COM-initialized worker thread.
class PcmDecoder {
 public:
  PcmDecoder();
  ~PcmDecoder();
  PcmDecoder(const PcmDecoder&) = delete;
  PcmDecoder& operator=(const PcmDecoder&) = delete;
  bool open(const wchar_t* path);
  bool read(double start, double seconds, PcmWindow* output);
  void close();

 private:
  struct State;
  std::unique_ptr<State> state_;
};

// Windows correspond to clips by index. No decoding, allocation or device I/O.
void MixAudio(const std::vector<AudioClip>& clips,
              const std::vector<std::shared_ptr<const PcmWindow>>& windows,
              double timelineStart, UINT32 outputRate, UINT32 frames, float* stereo);

class AudioEngine {
 public:
  AudioEngine();
  ~AudioEngine();
  AudioEngine(const AudioEngine&) = delete;
  AudioEngine& operator=(const AudioEngine&) = delete;
  void setClips(std::vector<AudioClip> clips);
  void setTransport(double time, bool playing);
  double position() const;
  bool playing() const;
  bool available() const;
  void shutdown();

 private:
  struct State;
  std::unique_ptr<State> state_;
};
