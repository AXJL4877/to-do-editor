use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::protocol::{
    encode_aspect, encode_audio, encode_gizmo, encode_guides, encode_hwnd, encode_layers,
    encode_layout, encode_path, encode_path_token, encode_seek, encode_size_screen,
    encode_transform, encode_transport, encode_visual_transform, parse_host_line, ParsedLine,
    HOST_RELATIVE, MAX_STDOUT_LINE,
};
use crate::viewport::encode_viewport;
use crate::{
    AlignGuide, AudioClip, ClockSample, HostEvent, LayoutRect, PreviewBounds, PreviewError,
    PreviewLayer, PreviewPointerKind, ScreenSlot, TransformGizmo, VisualTransform,
};
use crate::{PreviewNavigationKind, PreviewViewport};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const EVENT_CAP: usize = 64;
const QUIT_GRACE: Duration = Duration::from_millis(500);
const WALK_LIMIT: usize = 8;

struct Session {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    reader: Option<JoinHandle<()>>,
    out: Vec<u8>,
    shut: bool,
}

struct HostState {
    clock: Option<ClockSample>,
    bounds: Option<PreviewBounds>,
    source_aspect: Option<HostEvent>,
    pointers: VecDeque<HostEvent>,
}

struct Shared {
    state: Mutex<HostState>,
    tx: SyncSender<HostEvent>,
}

/// Native preview process.
///
/// Drop sends `quit`, closes stdin, waits briefly, then kills the child.
/// Stdout is read on a background thread into a mutex (latest sample) and a
/// bounded channel (wakeups). If the channel is full, the mutex still holds
/// the latest `audio-clock` / `preview-bounds`. Pointer edges are retained
/// separately, with consecutive moves coalesced until the consumer polls.
/// The latest decoded source aspect is retained ahead of pointer input as well.
#[must_use = "dropping PreviewHost stops the preview process"]
pub struct PreviewHost {
    session: Mutex<Session>,
    shared: Arc<Shared>,
    events: Mutex<Receiver<HostEvent>>,
}

impl PreviewHost {
    /// Spawn `TODO_PREVIEW_HOST` when that file exists, otherwise walk upward
    /// from the working directory and from this crate for [`HOST_RELATIVE`].
    pub fn spawn() -> Result<Self, PreviewError> {
        Self::spawn_at(resolve_host_exe()?)
    }

    /// Spawn a specific `preview_host.exe`. Its directory becomes the child cwd.
    pub fn spawn_at(exe: impl AsRef<Path>) -> Result<Self, PreviewError> {
        let exe = exe.as_ref();
        if !exe.is_file() {
            return Err(PreviewError::HostNotFound(exe.to_path_buf()));
        }
        let mut cmd = Command::new(exe);
        if let Some(dir) = exe.parent() {
            if !dir.as_os_str().is_empty() {
                cmd.current_dir(dir);
            }
        }
        spawn_child(cmd, Stdio::inherit())
    }

    /// `hwnd <hex>` with no `0x` prefix, matching Electron `toString(16)`.
    pub fn hwnd(&self, hwnd: u64) -> Result<(), PreviewError> {
        self.send(|buf| {
            encode_hwnd(buf, hwnd);
            Ok(())
        })
    }

    /// `path <media>` or `path` when `path` is `None` or empty.
    pub fn set_path(&self, path: Option<&str>) -> Result<(), PreviewError> {
        self.send(|buf| encode_path(buf, path))
    }

    /// Request a source with a caller token. Its accepted decoded frame reports
    /// `SourceAspect` with the same generation; `None` or empty clears it.
    pub fn set_path_with_generation(
        &self,
        path: Option<&str>,
        generation: u64,
    ) -> Result<(), PreviewError> {
        self.send(|buf| encode_path_token(buf, path, generation))
    }

    /// `seek <seconds>`. Negative values are clamped to 0.
    pub fn seek(&self, seconds: f64) -> Result<(), PreviewError> {
        self.send(|buf| encode_seek(buf, seconds))
    }

    /// `aspect <width> <height>`. Both must be finite and > 0.
    pub fn aspect(&self, width: f64, height: f64) -> Result<(), PreviewError> {
        self.send(|buf| encode_aspect(buf, width, height))
    }

    /// Move the fitted visual by canvas fractions and scale around its center.
    /// Defaults are `(0, 0, 1)`. All values must be finite and scale > 0.
    pub fn transform(&self, x: f64, y: f64, scale: f64) -> Result<(), PreviewError> {
        self.send(|buf| encode_transform(buf, x, y, scale))
    }

    /// Set placement, independent axis scale and opacity; redraws the retained frame.
    pub fn set_transform(&self, transform: VisualTransform) -> Result<(), PreviewError> {
        self.send(|buf| encode_visual_transform(buf, transform))
    }

    /// Show normalized selection bounds; `None` removes the selection overlay.
    pub fn set_gizmo(&self, gizmo: Option<TransformGizmo>) -> Result<(), PreviewError> {
        self.send(|buf| encode_gizmo(buf, gizmo))
    }

    /// Draw smart-align guides; empty slice clears.
    pub fn set_guides(&self, guides: &[AlignGuide]) -> Result<(), PreviewError> {
        self.send(|buf| encode_guides(buf, guides))
    }

    /// Change the editing camera and clipping; redraws retained frames without decoding.
    pub fn set_viewport(&self, viewport: PreviewViewport) -> Result<(), PreviewError> {
        self.send(|buf| encode_viewport(buf, viewport))
    }

    /// `layout <x> <y> <w> <h>`.
    pub fn layout(&self, rect: LayoutRect) -> Result<(), PreviewError> {
        self.send(|buf| encode_layout(buf, rect))
    }

    /// `size-screen <x> <y> <w> <h> <visible>`.
    pub fn size_screen(&self, slot: ScreenSlot) -> Result<(), PreviewError> {
        self.send(|buf| encode_size_screen(buf, slot))
    }

    /// `audio-begin`, one `audio-clip` line per clip, then `audio-commit`.
    pub fn set_audio(&self, clips: &[AudioClip]) -> Result<(), PreviewError> {
        self.send(|buf| encode_audio(buf, clips))
    }

    /// `layers-begin` / `layer`… / `layers-commit`. Empty clears the picture stack.
    pub fn set_layers(&self, layers: &[PreviewLayer]) -> Result<(), PreviewError> {
        self.send(|buf| encode_layers(buf, layers))
    }

    /// `transport <seconds> <playing 0|1>`. Negative time is clamped to 0.
    pub fn transport(&self, time: f64, playing: bool) -> Result<(), PreviewError> {
        self.send(|buf| encode_transport(buf, time, playing))
    }

    /// Send `quit` and wait until the process is gone.
    pub fn quit(&self) -> Result<(), PreviewError> {
        self.shutdown()
    }

    /// Latest sample, including ones older than one second.
    pub fn clock_sample(&self) -> Option<ClockSample> {
        lock(&self.shared.state).clock
    }

    /// Electron `readAudioClock` at `Instant::now()`.
    pub fn read_clock(&self) -> Option<crate::AudioClock> {
        self.clock_sample()?.extrapolated(Instant::now())
    }

    /// Latest `preview-bounds`, if the host has printed one.
    pub fn preview_bounds(&self) -> Option<PreviewBounds> {
        lock(&self.shared.state).bounds
    }

    /// Next stdout event. Decoded geometry arrives before captured pointer
    /// input so the first gesture can use the current source aspect. Empty and
    /// disconnected both return `None`. Consecutive pending moves coalesce.
    /// [`Self::clock_sample`] and [`Self::preview_bounds`] stay current either way.
    pub fn try_recv(&self) -> Option<HostEvent> {
        {
            let mut state = lock(&self.shared.state);
            if let Some(aspect) = state.source_aspect.take() {
                return Some(aspect);
            }
            if let Some(pointer) = state.pointers.pop_front() {
                return Some(pointer);
            }
        }
        lock(&self.events).try_recv().ok()
    }

    pub fn is_running(&self) -> bool {
        let mut session = lock(&self.session);
        if session.shut {
            return false;
        }
        matches!(session.child.try_wait(), Ok(None))
    }

    fn send(
        &self,
        encode: impl FnOnce(&mut Vec<u8>) -> Result<(), PreviewError>,
    ) -> Result<(), PreviewError> {
        let mut session = lock(&self.session);
        if session.shut || session.stdin.is_none() {
            return Err(PreviewError::Closed);
        }
        match session.child.try_wait() {
            Ok(Some(_)) => {
                session.stdin.take();
                return Err(PreviewError::Closed);
            }
            Ok(None) => {}
            Err(err) => return Err(err.into()),
        }
        session.out.clear();
        encode(&mut session.out)?;
        let write_result = {
            let Session { stdin, out, .. } = &mut *session;
            match stdin.as_mut() {
                Some(stdin) => stdin.write_all(out),
                None => Ok(()),
            }
        };
        if write_result.is_err() {
            session.stdin.take();
        }
        write_result.map_err(map_io)
    }

    fn shutdown(&self) -> Result<(), PreviewError> {
        let mut session = lock(&self.session);
        if session.shut {
            return Ok(());
        }
        session.shut = true;
        let write_result = {
            let Session { stdin, .. } = &mut *session;
            match stdin.as_mut() {
                Some(stdin) => stdin.write_all(b"quit\n"),
                None => Ok(()),
            }
        };
        // Closing stdin is the host's EOF (`WM_NATIVE_EOF`) if `quit` is missed.
        session.stdin.take();
        reap(&mut session.child, QUIT_GRACE);
        if let Some(reader) = session.reader.take() {
            let _ = reader.join();
        }
        write_result.map_err(map_io)
    }
}

impl Drop for PreviewHost {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl std::fmt::Debug for PreviewHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewHost")
            .field("running", &self.is_running())
            .finish()
    }
}

/// Locate the host executable.
///
/// `TODO_PREVIEW_HOST` wins when it is set and non-empty. A missing override
/// is an error. Otherwise search `native/build/bin/preview_host.exe` by
/// walking up from the process cwd and from this crate.
pub fn resolve_host_exe() -> Result<PathBuf, PreviewError> {
    let env = std::env::var_os("TODO_PREVIEW_HOST");
    let cwd = std::env::current_dir().ok();
    resolve_host(
        env.as_deref(),
        cwd.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

pub(crate) fn resolve_host(
    env_override: Option<&std::ffi::OsStr>,
    cwd: Option<&Path>,
    manifest_dir: &Path,
) -> Result<PathBuf, PreviewError> {
    if let Some(raw) = env_override {
        if !raw.is_empty() {
            let path = PathBuf::from(raw);
            if path.is_file() {
                return Ok(path);
            }
            return Err(PreviewError::HostNotFound(path));
        }
    }
    let rel = Path::new(HOST_RELATIVE);
    if let Some(cwd) = cwd {
        if let Some(found) = walk_up(cwd, rel) {
            return Ok(found);
        }
    }
    if let Some(found) = walk_up(manifest_dir, rel) {
        return Ok(found);
    }
    // crates/todo_preview -> repo root.
    let hint = manifest_dir.join("../..").join(rel);
    Err(PreviewError::HostNotFound(hint))
}

fn walk_up(start: &Path, rel: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    for _ in 0..=WALK_LIMIT {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn spawn_child(mut cmd: Command, stderr: Stdio) -> Result<PreviewHost, PreviewError> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("preview host stdin was not piped").into());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("preview host stdout was not piped").into());
        }
    };
    let (tx, rx) = mpsc::sync_channel(EVENT_CAP);
    let shared = Arc::new(Shared {
        state: Mutex::new(HostState {
            clock: None,
            bounds: None,
            source_aspect: None,
            pointers: VecDeque::new(),
        }),
        tx,
    });
    let shared_reader = Arc::clone(&shared);
    let reader = match thread::Builder::new()
        .name("preview-host-stdout".into())
        .spawn(move || reader_main(stdout, shared_reader))
    {
        Ok(handle) => handle,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err.into());
        }
    };
    Ok(PreviewHost {
        session: Mutex::new(Session {
            child,
            stdin: Some(stdin),
            reader: Some(reader),
            out: Vec::with_capacity(256),
            shut: false,
        }),
        shared,
        events: Mutex::new(rx),
    })
}

fn reader_main(stdout: ChildStdout, shared: Arc<Shared>) {
    let mut reader = BufReader::with_capacity(4096, stdout);
    let mut buf = Vec::with_capacity(128);
    let _ = pump(&mut reader, &mut buf, &shared);
}

fn pump<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>, shared: &Shared) -> std::io::Result<()> {
    while read_frame(reader, buf)? {
        if buf.is_empty() {
            continue;
        }
        let Ok(text) = std::str::from_utf8(buf) else {
            continue;
        };
        let Some(parsed) = parse_host_line(text.trim_end_matches(['\n', '\r'])) else {
            continue;
        };
        let event = match parsed {
            ParsedLine::Navigation(input) => HostEvent::PreviewNavigation(input),
            ParsedLine::Clock {
                time,
                playing,
                available,
            } => HostEvent::AudioClock(ClockSample {
                time,
                playing,
                available,
                received_at: Instant::now(),
            }),
            ParsedLine::Bounds(bounds) => HostEvent::PreviewBounds(bounds),
            ParsedLine::SourceAspect { generation, aspect } => {
                HostEvent::SourceAspect { generation, aspect }
            }
            ParsedLine::Pointer { kind, x, y } => HostEvent::PreviewPointer { kind, x, y },
        };
        {
            let mut state = lock(&shared.state);
            match event {
                HostEvent::AudioClock(sample) => state.clock = Some(sample),
                HostEvent::PreviewBounds(bounds) => state.bounds = Some(bounds),
                HostEvent::SourceAspect { .. } => state.source_aspect = Some(event),
                HostEvent::PreviewNavigation(input) => {
                    if input.kind == PreviewNavigationKind::PanMove
                        && matches!(state.pointers.back(), Some(HostEvent::PreviewNavigation(last))
                                    if last.kind == PreviewNavigationKind::PanMove)
                    {
                        state.pointers.pop_back();
                    }
                    state.pointers.push_back(event);
                }
                HostEvent::PreviewPointer { kind, .. } => {
                    // Pointer edges cannot be dropped when clock wakeups fill
                    // the bounded channel. Only consecutive motion coalesces.
                    if kind == PreviewPointerKind::Move
                        && matches!(
                            state.pointers.back(),
                            Some(HostEvent::PreviewPointer {
                                kind: PreviewPointerKind::Move,
                                ..
                            })
                        )
                    {
                        state.pointers.pop_back();
                    }
                    state.pointers.push_back(event);
                }
            }
        }
        if !matches!(
            event,
            HostEvent::PreviewPointer { .. }
                | HostEvent::SourceAspect { .. }
                | HostEvent::PreviewNavigation(_)
        ) {
            let _ = shared.tx.try_send(event);
        }
    }
    Ok(())
}

/// Read one newline-terminated frame. Overlong frames are discarded.
/// A trailing partial line at EOF is dropped, matching Electron's newline split.
fn read_frame<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    loop {
        let (newline, n) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                buf.clear();
                return Ok(false);
            }
            (
                available.iter().position(|&byte| byte == b'\n'),
                available.len(),
            )
        };
        if let Some(i) = newline {
            if buf.len() + i > MAX_STDOUT_LINE {
                reader.consume(i + 1);
                buf.clear();
                return Ok(true);
            }
            {
                let available = reader.fill_buf()?;
                buf.extend_from_slice(&available[..=i]);
            }
            reader.consume(i + 1);
            return Ok(true);
        }
        if buf.len() + n > MAX_STDOUT_LINE {
            reader.consume(n);
            buf.clear();
            return discard_line(reader);
        }
        {
            let available = reader.fill_buf()?;
            buf.extend_from_slice(&available[..n]);
        }
        reader.consume(n);
    }
}

fn discard_line<R: BufRead>(reader: &mut R) -> std::io::Result<bool> {
    loop {
        let (newline, n) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(false);
            }
            (
                available.iter().position(|&byte| byte == b'\n'),
                available.len(),
            )
        };
        if let Some(i) = newline {
            reader.consume(i + 1);
            return Ok(true);
        }
        reader.consume(n);
    }
}

fn reap(child: &mut Child, grace: Duration) {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

fn map_io(err: std::io::Error) -> PreviewError {
    if err.kind() == std::io::ErrorKind::BrokenPipe {
        PreviewError::Closed
    } else {
        PreviewError::Io(err)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests;
