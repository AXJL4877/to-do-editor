#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <iostream>
#include <mutex>
#include <string>
#include <thread>
#include <sstream>
#include <vector>
#include <unordered_map>
#include <io.h>
#include <fcntl.h>
#include <wincodec.h>
#include <wrl/client.h>

#include "video.h"
#include "presenter.h"
#include "transform/gizmo_cursor.h"
#include "transform/source_request.h"
#include "transform/frame_display_geometry.h"
#include "audio.h"

static HWND g_popup = nullptr;
static HWND g_owner = nullptr;
static HWND g_msg = nullptr;
static std::wstring g_media_path;
static double g_seek_sec = 0;
static uint64_t g_source_generation = 0;
static std::optional<double> g_reported_source_aspect;
static VideoEngine g_video;
static std::shared_ptr<const std::vector<VideoFrame>> g_frames;
static GpuPresenter g_presenter;
static AudioEngine g_audio;
static std::vector<AudioClip> g_pending_audio;
static bool g_audio_configuring = false;
static std::vector<LayerSpec> g_pending_layers;
static bool g_layers_configuring = false;

static LayerLook LookOf(VideoFrame::Kind kind, double time, double soft);

static bool PresentStack() {
  if (!g_popup || !g_frames || g_frames->empty()) {
    if (g_popup) g_presenter.clear();
    return false;
  }
  std::vector<DecoderFrame> frames;
  std::vector<VisualTransform> transforms;
  std::vector<LayerLook> looks;
  frames.reserve(g_frames->size());
  transforms.reserve(g_frames->size());
  looks.reserve(g_frames->size());
  for (const auto& item : *g_frames) {
    frames.push_back(item.frame);
    transforms.push_back(item.transform);
    looks.push_back(LookOf(item.kind, item.target, item.soft));
  }
  return g_presenter.presentLayers(frames.data(), transforms.data(), frames.size(), looks.data());
}
static constexpr wchar_t kClassName[] = L"TodoPreviewHost";
static constexpr wchar_t kMsgClass[] = L"TodoPreviewMsg";
static constexpr UINT WM_NATIVE_CMD = WM_APP + 1;
static constexpr UINT WM_NATIVE_EOF = WM_APP + 2;
static constexpr UINT WM_VIDEO_FRAME = WM_APP + 3;

static std::mutex g_mu;
static std::vector<std::string> g_cmds;
static bool g_pointer_down = false;
static bool g_pan_down = false;
static double g_pointer_x = 0;
static double g_pointer_y = 0;

static void EmitPointer(const char* kind) {
  printf("preview-pointer %s %.9f %.9f\n", kind, g_pointer_x, g_pointer_y);
  fflush(stdout);
}

static void EmitNavigation(const char* kind, double delta = 0) {
  printf("preview-navigation %s %.9f %.9f %.9f\n", kind, delta, g_pointer_x, g_pointer_y);
  fflush(stdout);
}

static void UpdatePointer(HWND hwnd, LPARAM position) {
  RECT rc{};
  GetClientRect(hwnd, &rc);
  // Signed client coordinates preserve motion beyond the canvas during capture.
  g_pointer_x = static_cast<short>(LOWORD(position)) /
                static_cast<double>((std::max)(1L, rc.right - rc.left));
  g_pointer_y = static_cast<short>(HIWORD(position)) /
                static_cast<double>((std::max)(1L, rc.bottom - rc.top));
}

static void FinishPointer(HWND hwnd, bool release) {
  if (!g_pointer_down) return;
  g_pointer_down = false;
  EmitPointer("up");
  if (release && GetCapture() == hwnd) ReleaseCapture();
}

static void FinishPan(HWND hwnd, bool release) {
  if (!g_pan_down) return;
  g_pan_down = false;
  EmitNavigation("pan-up");
  if (release && GetCapture() == hwnd) ReleaseCapture();
}

static void PresentOrPaint(HWND hwnd) {
  if (g_frames && !g_frames->empty() && PresentStack()) {
    ValidateRect(hwnd, nullptr);
    return;
  }
  PAINTSTRUCT ps{};
  HDC hdc = BeginPaint(hwnd, &ps);
  RECT rc{};
  GetClientRect(hwnd, &rc);
  HBRUSH brush = CreateSolidBrush(RGB(12, 12, 12));
  FillRect(hdc, &rc, brush);
  DeleteObject(brush);
  SetBkMode(hdc, TRANSPARENT);
  SetTextColor(hdc, RGB(160, 160, 160));
  wchar_t line[512];
  if (g_media_path.empty() && (!g_frames || g_frames->empty())) {
    swprintf_s(line, L"HWND host  seek=%.3f", g_seek_sec);
  } else {
    swprintf_s(line, L"seek=%.3f  (decode pending/fail)", g_seek_sec);
  }
  DrawTextW(hdc, line, -1, &rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
  EndPaint(hwnd, &ps);
}

static LRESULT CALLBACK PreviewWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  switch (msg) {
    case WM_SETCURSOR:
      if (LOWORD(lParam) == HTCLIENT) {
        POINT point{};
        GetCursorPos(&point);
        ScreenToClient(hwnd, &point);
        SetCursor(g_pan_down ? LoadCursor(nullptr, IDC_HAND) :
                  TransformGizmoCursor(hwnd, g_presenter.gizmo(), point));
        return TRUE;
      }
      break;
    case WM_MOUSEACTIVATE:
      return MA_NOACTIVATE;
    case WM_LBUTTONDOWN:
      if (!g_pointer_down && !g_pan_down) {
        UpdatePointer(hwnd, lParam);
        SetCapture(hwnd);
        g_pointer_down = true;
        EmitPointer("down");
      }
      return 0;
    case WM_MBUTTONDOWN:
      if (!g_pointer_down && !g_pan_down) {
        UpdatePointer(hwnd, lParam);
        SetCapture(hwnd);
        g_pan_down = true;
        EmitNavigation("pan-down");
      }
      return 0;
    case WM_MBUTTONUP:
      if (g_pan_down) {
        UpdatePointer(hwnd, lParam);
        FinishPan(hwnd, true);
      }
      return 0;
    case WM_MOUSEWHEEL: {
      if (g_pointer_down || g_pan_down) return 0;
      POINT point{static_cast<short>(LOWORD(lParam)), static_cast<short>(HIWORD(lParam))};
      ScreenToClient(hwnd, &point);
      UpdatePointer(hwnd, MAKELPARAM(point.x, point.y));
      EmitNavigation("wheel", static_cast<short>(HIWORD(wParam)) / static_cast<double>(WHEEL_DELTA));
      return 0;
    }
    case WM_MOUSEMOVE:
      SetCursor(g_pan_down ? LoadCursor(nullptr, IDC_HAND) : TransformGizmoCursor(hwnd, g_presenter.gizmo(),
                {static_cast<short>(LOWORD(lParam)), static_cast<short>(HIWORD(lParam))}));
      if (g_pointer_down) {
        UpdatePointer(hwnd, lParam);
        EmitPointer("move");
      } else if (g_pan_down) {
        UpdatePointer(hwnd, lParam);
        EmitNavigation("pan-move");
      }
      return 0;
    case WM_LBUTTONUP:
      if (g_pointer_down) {
        UpdatePointer(hwnd, lParam);
        FinishPointer(hwnd, true);
      }
      return 0;
    case WM_CAPTURECHANGED:
      FinishPointer(hwnd, false);
      FinishPan(hwnd, false);
      return 0;
    case WM_CANCELMODE:
      FinishPointer(hwnd, true);
      FinishPan(hwnd, true);
      return 0;
    case WM_SHOWWINDOW:
      if (!wParam) { FinishPointer(hwnd, true); FinishPan(hwnd, true); }
      break;
    case WM_DESTROY:
      FinishPointer(hwnd, true);
      FinishPan(hwnd, true);
      return 0;
    case WM_SIZE: {
      const int w = LOWORD(lParam);
      const int h = HIWORD(lParam);
      if (w > 0 && h > 0) g_presenter.resize(w, h);
      PresentStack();
      return 0;
    }
    case WM_DPICHANGED:
      // Handle dimensions follow window DPI even when the physical slot size
      // stays unchanged. The app still owns placement of the preview popup.
      PresentStack();
      return 0;
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT:
      PresentOrPaint(hwnd);
      return 0;
    default:
      break;
  }
  return DefWindowProcW(hwnd, msg, wParam, lParam);
}

static void EnsurePreviewClass() {
  static bool ready = false;
  if (ready) return;
  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.lpfnWndProc = PreviewWndProc;
  wc.hInstance = GetModuleHandleW(nullptr);
  wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
  wc.lpszClassName = kClassName;
  RegisterClassExW(&wc);
  ready = true;
}

static void BindPresenterIfReady() {
  if (!g_popup || !IsWindow(g_popup)) return;
  if (!g_frames || g_frames->empty()) return;
  const auto& top = g_frames->front();
  if (!g_presenter.bind(g_popup, top.device.Get(), top.context.Get())) return;
  RECT rc{};
  GetClientRect(g_popup, &rc);
  g_presenter.resize((std::max)(1L, rc.right - rc.left), (std::max)(1L, rc.bottom - rc.top));
}

// Owner only (z-order / lifetime). The preview itself is a top-level WS_POPUP,
// positioned in physical screen pixels. It is not a child of the content HWND.
static void CmdHwnd(const std::string& arg) {
  unsigned long long value = 0;
  if (sscanf_s(arg.c_str(), "%llx", &value) != 1) return;
  HWND frame = reinterpret_cast<HWND>(static_cast<uintptr_t>(value));
  if (!frame || !IsWindow(frame)) return;
  if (g_owner == frame) return;
  g_owner = frame;
  if (g_popup && IsWindow(g_popup)) {
    DestroyWindow(g_popup);
    g_popup = nullptr;
  }
}

// x,y,w,h are physical screen pixels from Electron screen.dipToScreenRect.
static void PlacePopup(int x, int y, int w, int h, int visible) {
  w = (std::max)(1, w);
  h = (std::max)(1, h);
  EnsurePreviewClass();
  if (!g_popup || !IsWindow(g_popup)) {
    g_popup = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        kClassName,
        L"TodoPreview",
        WS_POPUP,
        x, y, w, h,
        g_owner,
        nullptr,
        GetModuleHandleW(nullptr),
        nullptr);
  }
  if (!g_popup) return;
  const bool show = visible && w > 1 && h > 1;
  SetWindowPos(
      g_popup,
      HWND_TOP,
      x, y, w, h,
      SWP_NOACTIVATE | (show ? SWP_SHOWWINDOW : SWP_HIDEWINDOW));
  BindPresenterIfReady();
  g_presenter.resize(w, h);
  if (!PresentStack()) g_presenter.clear();
  InvalidateRect(g_popup, nullptr, TRUE);
}

static void CmdPath(const std::string& arg, uint64_t generation = 0, bool tagged = false) {
  if (arg.empty()) {
    g_media_path.clear();
    g_source_generation = generation;
    g_reported_source_aspect.reset();
    g_video.setLayers({}, generation);
    g_frames.reset();
    g_presenter.setGizmo(std::nullopt);
    g_presenter.setGuides({});
    if (g_popup) g_presenter.clear();
    return;
  }
  const int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, arg.data(),
                                  static_cast<int>(arg.size()), nullptr, 0);
  if (n <= 0) return;
  std::wstring path(static_cast<size_t>(n), L'\0');
  MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, arg.data(),
                      static_cast<int>(arg.size()), path.data(), n);
  // Same path with a live frame: keep showing it (stills and video). Protocol
  // still uses path/seek; stills simply republish frame 0 on seek.
  if (!tagged && path == g_media_path && g_frames && !g_frames->empty() &&
      g_source_generation == generation)
    return;
  g_source_generation = generation;
  g_reported_source_aspect.reset();
  g_media_path = std::move(path);
  g_frames.reset();
  g_presenter.setGizmo(std::nullopt);
  g_presenter.setGuides({});
  VisualTransform transform;
  g_presenter.setTransform(transform);
  g_video.setLayers({LayerSpec{VideoFrame::Kind::Video, g_media_path, g_seek_sec, transform}}, generation);
  if (g_popup) g_presenter.clear();
}

static void CmdPathToken(const std::string& arg) {
  uint64_t generation = 0;
  std::string path;
  if (ParseSourceRequest(arg, generation, path)) CmdPath(path, generation, true);
}

static void CmdSeek(const std::string& arg) {
  double seconds = 0;
  std::istringstream input(arg);
  if (!(input >> seconds) || !std::isfinite(seconds)) return;
  g_seek_sec = (std::max)(0.0, seconds);
  // Images ignore the time and keep displaying the decoded still.
  g_video.seek(g_seek_sec);
}

static void CmdTransform(const std::string& arg) {
  VisualTransform transform;
  if (!ParseVisualTransform(arg, transform) || !g_presenter.setTransform(transform)) return;
  // Prefer updating the live single-source stack so decode textures are reused.
  if (!g_media_path.empty()) {
  g_video.setLayers({LayerSpec{VideoFrame::Kind::Video, g_media_path, g_seek_sec, transform}}, g_source_generation);
    return;
  }
  if (g_popup && g_frames && g_frames->size() == 1) {
    auto updated = std::make_shared<std::vector<VideoFrame>>(*g_frames);
    (*updated)[0].transform = transform;
    g_frames = std::move(updated);
    PresentStack();
  }
}

static void CmdGizmo(const std::string& arg) {
  std::optional<TransformGizmo> gizmo;
  if (!ParseTransformGizmo(arg, gizmo) || !g_presenter.setGizmo(gizmo)) return;
  PresentStack();
}

static void CmdGuides(const std::string& arg) {
  std::vector<AlignGuide> guides;
  if (!ParseAlignGuides(arg, guides) || !g_presenter.setGuides(std::move(guides))) return;
  PresentStack();
}

static void CmdViewport(const std::string& arg) {
  PreviewViewport viewport;
  if (!ParsePreviewViewport(arg, viewport) || !g_presenter.setViewport(viewport)) return;
  PresentStack();
}

static bool Utf8ToWide(const std::string& text, std::wstring* out) {
  if (!out) return false;
  const int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(),
                                    static_cast<int>(text.size()), nullptr, 0);
  if (n <= 0) return false;
  out->assign(static_cast<size_t>(n), L'\0');
  MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text.data(), static_cast<int>(text.size()),
                      out->data(), n);
  return true;
}

// `<kind> <time> <x> <y> <scale> <scale_x> <scale_y> <opacity> <path>`; shared by
// the window preview and the offline renderer.
static bool ParseLayer(const std::string& arg, LayerSpec* out) {
  LayerSpec layer;
  std::istringstream input(arg);
  std::string kind;
  if (!(input >> kind >> layer.time >> layer.transform.x >> layer.transform.y >> layer.transform.scale >>
        layer.transform.scale_x >> layer.transform.scale_y >> layer.transform.opacity))
    return false;
  if (kind == "video") layer.kind = VideoFrame::Kind::Video;
  else if (kind == "image") layer.kind = VideoFrame::Kind::Image;
  else if (kind == "packed") layer.kind = VideoFrame::Kind::Packed;
  else if (kind.rfind("reveal", 0) == 0) {
    // `reveal:<soft>`; the media time is the progress 0..1.
    layer.kind = VideoFrame::Kind::Reveal;
    if (kind.size() > 7 && kind[6] == ':') {
      try {
        layer.soft = std::stod(kind.substr(7));
      } catch (...) {
        return false;
      }
    }
    if (!std::isfinite(layer.soft) || layer.soft < 0) return false;
  } else if (kind == "text") layer.kind = VideoFrame::Kind::Text;
  else return false;
  if (!std::isfinite(layer.time) || layer.time < 0 || !layer.transform.valid()) return false;
  std::string path;
  std::getline(input >> std::ws, path);
  if (!Utf8ToWide(path, &layer.path) || layer.path.empty()) return false;
  *out = std::move(layer);
  return true;
}

static LayerLook LookOf(VideoFrame::Kind kind, double time, double soft) {
  LayerLook look;
  if (kind == VideoFrame::Kind::Packed) look.packing = LayerPacking::Packed;
  if (kind == VideoFrame::Kind::Reveal) {
    look.packing = LayerPacking::Reveal;
    look.progress = static_cast<float>(time);
    look.soft = static_cast<float>(soft);
  }
  return look;
}

static void CmdLayer(const std::string& arg) {
  if (!g_layers_configuring || g_pending_layers.size() >= VideoEngine::kMaxLayers) return;
  LayerSpec layer;
  if (ParseLayer(arg, &layer)) g_pending_layers.push_back(std::move(layer));
}

static bool RenderPathAvailable(const std::wstring& path) {
  bool available = true;
  // Querying a disconnected drive can block while the shell retries the
  // device. The logical-drive bitmask is immediate and lets the protocol
  // reject that case before Media Foundation or D3D is initialized.
  if (path.size() >= 3 && path[1] == L':' && (path[2] == L'\\' || path[2] == L'/')) {
    wchar_t drive = path[0];
    if (drive >= L'a' && drive <= L'z') drive = static_cast<wchar_t>(drive - L'a' + L'A');
    const DWORD logical_drives = GetLogicalDrives();
    available = drive >= L'A' && drive <= L'Z' && logical_drives != 0 &&
                (logical_drives & (1u << (drive - L'A'))) != 0;
    if (available) {
      wchar_t root[] = {drive, L':', L'\\', L'\0'};
      // A mapped network drive may be present in the bitmask while its server
      // is offline. Treat it as unavailable for the synchronous render
      // protocol rather than blocking on a network filesystem probe.
      const UINT drive_type = GetDriveTypeW(root);
      available = drive_type != DRIVE_REMOTE && drive_type != DRIVE_NO_ROOT_DIR;
    }
  }
  if (!available) return false;
  const DWORD attributes = GetFileAttributesW(path.c_str());
  return attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY) == 0;
}

static void CmdLayersCommit() {
  if (!g_layers_configuring) return;
  g_layers_configuring = false;
  g_source_generation += 1;
  g_reported_source_aspect.reset();
  if (g_pending_layers.empty()) {
    g_media_path.clear();
    g_frames.reset();
    g_presenter.setGizmo(std::nullopt);
    g_presenter.setGuides({});
    g_video.setLayers({}, g_source_generation);
    if (g_popup) g_presenter.clear();
  } else {
    g_media_path = g_pending_layers.back().path;
    g_seek_sec = g_pending_layers.back().time;
    if (!g_pending_layers.empty()) {
      g_presenter.setTransform(g_pending_layers.back().transform);
    }
    // Replace the visible stack only after the complete new stack is ready.
    // Transform gestures also arrive through this path on every pointer move.
    g_video.setLayers(std::move(g_pending_layers), g_source_generation);
  }
  g_pending_layers.clear();
}

static void CmdSizeScreen(const std::string& arg) {
  int x = 0, y = 0, w = 0, h = 0, visible = 1;
  if (sscanf_s(arg.c_str(), "%d %d %d %d %d", &x, &y, &w, &h, &visible) < 4) return;
  PlacePopup(x, y, w, h, visible);
}

static void CmdAudioClip(const std::string& arg) {
  if (!g_audio_configuring || g_pending_audio.size() >= 4096) return;
  AudioClip clip;
  std::istringstream input(arg);
  if (!(input >> clip.start >> clip.duration >> clip.offset >> clip.rate >> clip.gain >> clip.fade_in >> clip.fade_out)) return;
  if (!std::isfinite(clip.start) || !std::isfinite(clip.duration) ||
      !std::isfinite(clip.offset) || !std::isfinite(clip.rate) || !std::isfinite(clip.gain) ||
      !std::isfinite(clip.fade_in) || !std::isfinite(clip.fade_out) ||
      clip.start < 0 || clip.duration <= 0 || clip.offset < 0 ||
      clip.rate < 0.1 || clip.rate > 8 || clip.gain < 0 || clip.gain > 4 ||
      clip.fade_in < 0 || clip.fade_out < 0 || clip.fade_in + clip.fade_out > clip.duration) return;
  std::string path;
  std::getline(input >> std::ws, path);
  const int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path.data(),
                                  static_cast<int>(path.size()), nullptr, 0);
  if (n <= 0) return;
  clip.path.resize(n);
  MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path.data(),
                      static_cast<int>(path.size()), clip.path.data(), n);
  g_pending_audio.push_back(std::move(clip));
}

static void CmdTransport(const std::string& arg) {
  double time = 0;
  int playing = 0;
  std::istringstream input(arg);
  if (!(input >> time >> playing) || !std::isfinite(time)) return;
  g_audio.setTransport((std::max)(0.0, time), playing != 0);
}

static void HandleLine(const std::string& line) {
  if (line.empty()) return;
  const size_t sp = line.find(' ');
  const std::string cmd = sp == std::string::npos ? line : line.substr(0, sp);
  const std::string arg = sp == std::string::npos ? std::string() : line.substr(sp + 1);
  if (cmd == "hwnd") CmdHwnd(arg);
  else if (cmd == "path") CmdPath(arg);
  else if (cmd == "path-token") CmdPathToken(arg);
  else if (cmd == "seek") CmdSeek(arg);
  else if (cmd == "transform") CmdTransform(arg);
  else if (cmd == "gizmo") CmdGizmo(arg);
  else if (cmd == "guides") CmdGuides(arg);
  else if (cmd == "viewport") CmdViewport(arg);
  else if (cmd == "size-screen") CmdSizeScreen(arg);
  else if (cmd == "layers-begin") {
    g_pending_layers.clear();
    g_layers_configuring = true;
  } else if (cmd == "layer")
    CmdLayer(arg);
  else if (cmd == "layers-commit")
    CmdLayersCommit();
  else if (cmd == "audio-begin") {
    g_pending_audio.clear();
    g_audio_configuring = true;
  }
  else if (cmd == "audio-clip") CmdAudioClip(arg);
  else if (cmd == "audio-commit" && g_audio_configuring) {
    g_audio.setClips(std::move(g_pending_audio));
    g_pending_audio.clear();
    g_audio_configuring = false;
  }
  else if (cmd == "transport") CmdTransport(arg);
  else if (cmd == "quit") PostQuitMessage(0);
}

static LRESULT CALLBACK MsgWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  if (msg == WM_VIDEO_FRAME) {
    // latest() only returns frames of the currently requested sources. While
    // scrubbing they may belong to an older time of those sources (exact = false):
    // showing that progress beats freezing until the newest target decodes.
    // Ready messages can outlive their request. A pending replacement is not
    // an empty scene; explicit path/layers commands handle clearing the canvas.
    auto stack = g_video.latest();
    if (!stack || stack->empty() || g_frames == stack) return 0;
    const bool decoded = !g_frames || g_frames->empty() ||
                         g_frames->back().serial != stack->back().serial ||
                         g_frames->back().exact != stack->back().exact;
    g_frames = std::move(stack);
    const auto& top = g_frames->back();
    D3D11_TEXTURE2D_DESC texture{};
    top.frame.texture->GetDesc(&texture);
    auto geometry = DecodedFrameDisplayGeometry(top.frame, texture.Width, texture.Height);
    // The reported aspect is the picture's, not the packed texture's.
    if (geometry && top.kind == VideoFrame::Kind::Reveal) geometry->aspect /= 2.0;
    if (geometry && top.kind == VideoFrame::Kind::Packed) geometry->aspect *= 2.0;
    if (top.topmost && geometry &&
        (!g_reported_source_aspect || *g_reported_source_aspect != geometry->aspect)) {
      g_reported_source_aspect = geometry->aspect;
      // Same sources as the current request, so the aspect belongs to its generation.
      printf("source-aspect %llu %.17g\n", static_cast<unsigned long long>(g_source_generation),
             geometry->aspect);
      fflush(stdout);
    }
    BindPresenterIfReady();
    if (g_popup) {
      PresentStack();
      ValidateRect(g_popup, nullptr);
    }
    // Transform-only republishes redraw the same decode; keep the diagnostic
    // specific to decoding so tests can detect unnecessary decode requests.
    // `video-approx` reports scrub progress shown before the target decoded.
    if (decoded) {
      printf("%s %llu %.9f %.9f %.9f\n", top.exact ? "video-frame" : "video-approx",
             static_cast<unsigned long long>(top.serial), top.target, top.frame.timestamp,
             top.frame.duration);
      fflush(stdout);
    }
    return 0;
  }
  if (msg == WM_TIMER) {
    printf("audio-clock %.9f %d %d\n", g_audio.position(), g_audio.playing() ? 1 : 0,
           g_audio.available() ? 1 : 0);
    fflush(stdout);
    return 0;
  }
  if (msg == WM_NATIVE_CMD) {
    std::vector<std::string> batch;
    {
      std::lock_guard<std::mutex> lock(g_mu);
      batch.swap(g_cmds);
    }
    for (size_t i = 0; i < batch.size(); ++i) {
      // Drop superseded requests without crossing path/hwnd/quit command boundaries.
      const bool seek = batch[i].rfind("seek ", 0) == 0;
      const auto isSize = [](const std::string& line) {
        return line.rfind("size ", 0) == 0 || line.rfind("size-screen ", 0) == 0;
      };
      const bool size = isSize(batch[i]);
      if (i + 1 < batch.size() &&
          ((seek && batch[i + 1].rfind("seek ", 0) == 0) ||
           (size && isSize(batch[i + 1])))) continue;
      HandleLine(batch[i]);
    }
    return 0;
  }
  if (msg == WM_NATIVE_EOF) {
    PostQuitMessage(0);
    return 0;
  }
  return DefWindowProcW(hwnd, msg, wParam, lParam);
}

// Offline export renderer: the same decoders, layer protocol and shaders as the
// window preview, drawn offscreen. stdin:
//   begin <width> <height>
//   layers-begin / layer ... / layers-commit   (one frame; may be empty)
//   end
// Every commit writes width*height*4 BGRA bytes to stdout; frames are decoded
// exactly (never an approximate scrub frame).
static int RenderMain() {
  CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  _setmode(_fileno(stdout), _O_BINARY);
  int result = 0;
  {
    HwDecoder shared;
    GpuPresenter presenter;
    // Keep a bounded pool of decoder slots, but assign them by source path
    // instead of layer index. Timeline layers can be reordered between frames;
    // keeping the decoder with its source avoids reopening the same media and
    // preserves the per-slot frame lease needed when two layers share a path.
    std::vector<std::unique_ptr<HwDecoder>> slots(VideoEngine::kMaxLayers);
    std::vector<std::wstring> opened(VideoEngine::kMaxLayers);
    std::vector<uint64_t> last_used(VideoEngine::kMaxLayers, 0);
    uint64_t frame_serial = 0;
    std::vector<LayerSpec> layers;
    std::vector<unsigned char> pixels;
    int render_width = 0;
    int render_height = 0;
    bool device_ready = false;
    bool began = false, configuring = false, ended = false;
    std::string line;
    while (std::getline(std::cin, line)) {
      if (!line.empty() && line.back() == '\r') line.pop_back();
      if (line.rfind("begin ", 0) == 0) {
        std::istringstream input(line.substr(6));
        int width = 0, height = 0;
        if (!(input >> width >> height) || width < 2 || height < 2 || width > 8192 || height > 8192 ||
            (device_ready && !presenter.bindOffscreen(shared.device(), shared.context(), width, height))) {
          std::fputs("render: invalid begin\n", stderr);
          result = 5;
          break;
        }
        render_width = width;
        render_height = height;
        began = true;
      } else if (line == "layers-begin") {
        layers.clear();
        configuring = true;
      } else if (line.rfind("layer ", 0) == 0) {
        LayerSpec layer;
        if (!configuring || layers.size() >= VideoEngine::kMaxLayers || !ParseLayer(line.substr(6), &layer)) {
          std::fputs("render: invalid layer\n", stderr);
          result = 4;
          break;
        }
        layers.push_back(std::move(layer));
      } else if (line == "layers-commit") {
        if (!began || !configuring) {
          std::fputs("render: frame before begin\n", stderr);
          result = 2;
          break;
        }
        configuring = false;
        ++frame_serial;
        // Do cheap path validation before creating the GPU device. Besides
        // producing a prompt diagnostic, this keeps a disconnected drive from
        // consuming the renderer's startup/cancellation budget.
        for (size_t i = 0; i < layers.size(); ++i) {
          if (!RenderPathAvailable(layers[i].path)) {
            std::fprintf(stderr, "render: cannot open layer %zu\n", i);
            result = 3;
            break;
          }
        }
        if (result != 0) break;
        if (!device_ready) {
          if (!shared.prepareDevice() ||
              !presenter.bindOffscreen(shared.device(), shared.context(), render_width, render_height)) {
            std::fputs("render: cannot create the GPU device\n", stderr);
            result = 7;
            break;
          }
          device_ready = true;
        }
        std::vector<DecoderFrame> frames;
        std::vector<VisualTransform> transforms;
        std::vector<LayerLook> looks;
        std::vector<bool> used(VideoEngine::kMaxLayers, false);
        bool ok = true;
        for (size_t i = 0; i < layers.size() && ok; ++i) {
          const auto& layer = layers[i];
          size_t slot = VideoEngine::kMaxLayers;
          // Prefer an existing decoder for this path that has not already been
          // claimed by another occurrence in the current layer stack.
          for (size_t candidate = 0; candidate < slots.size(); ++candidate) {
            if (!used[candidate] && slots[candidate] && opened[candidate] == layer.path) {
              slot = candidate;
              break;
            }
          }
          if (slot == VideoEngine::kMaxLayers) {
            // Reuse an empty slot first, then evict the least recently used
            // slot that is not needed by this frame.
            for (size_t candidate = 0; candidate < slots.size(); ++candidate) {
              if (!used[candidate] && !slots[candidate]) {
                slot = candidate;
                break;
              }
            }
          }
          if (slot == VideoEngine::kMaxLayers) {
            uint64_t oldest = UINT64_MAX;
            for (size_t candidate = 0; candidate < slots.size(); ++candidate) {
              if (!used[candidate] && last_used[candidate] < oldest) {
                oldest = last_used[candidate];
                slot = candidate;
              }
            }
          }
          if (slot == VideoEngine::kMaxLayers) {
            std::fputs("render: decoder slot allocation failed\n", stderr);
            ok = false;
            break;
          }
          auto& decoder = slots[slot];
          if (!decoder || opened[slot] != layer.path) {
            decoder = std::make_unique<HwDecoder>();
            if (!decoder->attachDevice(shared.device(), shared.context(), shared.dxgiManager(),
                                       shared.resetToken()) ||
                !decoder->open(layer.path.c_str())) {
              std::fprintf(stderr, "render: cannot open layer %zu\n", i);
              ok = false;
              break;
            }
            opened[slot] = layer.path;
          }
          used[slot] = true;
          last_used[slot] = frame_serial;
          // A reveal's time is its progress; the atlas itself is a still.
          const double at = decoder->isStill() ? 0.0 : layer.time;
          double served = at;
          const SeekResult seek = decoder->seek(at, SeekControl{}, &served);
          if (seek != SeekResult::Exact || !decoder->frame()) {
            std::fprintf(stderr, "render: cannot decode layer %zu at %.6f\n", i, layer.time);
            ok = false;
            break;
          }
          frames.push_back(*decoder->frame());
          transforms.push_back(layer.transform);
          looks.push_back(LookOf(layer.kind, layer.time, layer.soft));
        }
        if (!ok || !presenter.renderOffscreen(frames.data(), transforms.data(), frames.size(), looks.data(),
                                              &pixels)) {
          if (ok) std::fputs("render: drawing failed\n", stderr);
          result = 3;
          break;
        }
        std::cout.write(reinterpret_cast<const char*>(pixels.data()), static_cast<std::streamsize>(pixels.size()));
        std::cout.flush();
        if (!std::cout) {
          result = 8;
          break;
        }
      } else if (line == "end") {
        ended = true;
        break;
      }
    }
    if (result == 0 && !began) result = 5;
    if (result == 0 && !ended) result = 6;
  }
  CoUninitialize();
  return result;
}

static void StdinLoop() {
  std::string line;
  for (;;) {
    if (!std::getline(std::cin, line)) break;
    if (!line.empty() && line.back() == '\r') line.pop_back();
    {
      std::lock_guard<std::mutex> lock(g_mu);
      g_cmds.push_back(line);
    }
    if (g_msg) PostMessageW(g_msg, WM_NATIVE_CMD, 0, 0);
    if (line == "quit" || line.rfind("quit ", 0) == 0) break;
  }
  if (g_msg) PostMessageW(g_msg, WM_NATIVE_EOF, 0, 0);
}

int main(int argc, char** argv) {
  if (argc > 1 && std::string(argv[1]) == "--render") return RenderMain();
  // Match Electron's Per-Monitor V2 awareness. An unaware process gets
  // virtualized coordinates, so the child HWND lands offset from the UI.
  SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
  SetConsoleOutputCP(CP_UTF8);
  CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.lpfnWndProc = MsgWndProc;
  wc.hInstance = GetModuleHandleW(nullptr);
  wc.lpszClassName = kMsgClass;
  RegisterClassExW(&wc);
  g_msg = CreateWindowExW(0, kMsgClass, L"", 0, 0, 0, 0, 0, HWND_MESSAGE, nullptr, wc.hInstance, nullptr);
  SetTimer(g_msg, 1, 50, nullptr);
  g_video.start(g_msg, WM_VIDEO_FRAME);

  std::thread(StdinLoop).detach();

  MSG msg{};
  while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
    TranslateMessage(&msg);
    DispatchMessageW(&msg);
  }

  KillTimer(g_msg, 1);
  g_frames.reset();
  g_video.shutdown();
  g_audio.shutdown();
  if (g_popup && IsWindow(g_popup)) DestroyWindow(g_popup);
  CoUninitialize();
  return 0;
}
