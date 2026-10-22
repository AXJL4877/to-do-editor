//! Embed a real Windows console into the workflow "terminal hole".
//!
//! egui reserves the bottom rect; we SetParent the console HWND into the app
//! window and size it in client coords. If SetParent is refused (some conhost
//! builds), we fall back to a chrome-less top-level popup over the same hole.
//! The console tails `run.log` — not an egui fake log.
use crate::adapters::desktop::window::{viewport_hwnd, ScreenLayout};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

const TITLE_PREFIX: &str = "TODO工作流终端";

const TAIL_SCRIPT: &str = r#"param([string]$LogPath, [string]$Title)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
try { $Host.UI.RawUI.WindowTitle = $Title } catch {}
Get-Content -LiteralPath $LogPath -Wait -Tail 200 -Encoding UTF8
"#;

fn tail_command(log_path: &Path, title: &str) -> Result<std::process::Command, String> {
    // canonicalize adds a verbatim \\?\ prefix; Windows PowerShell's -Wait
    // watcher rejects that prefix after printing the initial tail.
    let log_path = std::path::absolute(log_path).map_err(|e| e.to_string())?;
    let script_path = log_path.with_file_name("tail-log.ps1");
    std::fs::write(&script_path, TAIL_SCRIPT).map_err(|e| e.to_string())?;
    // Pass paths as arguments, never as a nested cmd/PowerShell expression.
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script_path).arg("-LogPath").arg(log_path).arg("-Title").arg(title);
    Ok(command)
}

pub struct WorkflowConsole {
    extension: Option<String>,
    log_path: Option<PathBuf>,
    title: String,
    hwnd: Option<isize>,
    /// True after SetParent succeeded — layout uses client coords.
    embedded: bool,
    child: Option<Child>,
    spawned_at: Option<Instant>,
    last_layout: Option<ScreenLayout>,
}

impl Default for WorkflowConsole {
    fn default() -> Self {
        Self {
            extension: None,
            log_path: None,
            title: TITLE_PREFIX.into(),
            hwnd: None,
            embedded: false,
            child: None,
            spawned_at: None,
            last_layout: None,
        }
    }
}

impl Drop for WorkflowConsole {
    fn drop(&mut self) {
        self.close();
    }
}

impl WorkflowConsole {
    /// Open (or rebind) the native console for this extension's `run.log`.
    /// Same extension + alive console is reused (stable log path) so opening
    /// the workflow does not flash a new cmd window.
    pub fn attach(&mut self, extension: &str, log_path: &Path) -> Result<(), String> {
        if !log_path.is_file() {
            return Err("日志文件还没生成".into());
        }
        if self.extension.as_deref() == Some(extension) && self.alive() {
            self.log_path = Some(log_path.to_path_buf());
            return Ok(());
        }
        self.close();
        let short = extension
            .rsplit(['.', '/', '\\'])
            .next()
            .unwrap_or(extension);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_nanos();
        self.title = format!("{TITLE_PREFIX} · {short} · {}-{stamp}", std::process::id());
        self.extension = Some(extension.to_owned());
        self.log_path = Some(log_path.to_path_buf());
        let tail = tail_command(log_path, &self.title)?;
        let child = std::process::Command::new("conhost.exe")
            .arg(tail.get_program()).args(tail.get_args())
            .creation_flags(0x00000010) // CREATE_NEW_CONSOLE
            .spawn()
            .map_err(|e| e.to_string())?;
        self.child = Some(child);
        self.spawned_at = Some(Instant::now());
        // Hide before the first paint so a free-floating cmd never flashes.
        self.hwnd = hide_new_console(&self.title, Duration::from_millis(800));
        self.embedded = false;
        Ok(())
    }

    pub fn hide(&mut self) {
        if let Some(hwnd) = self.hwnd {
            show_window(hwnd, false);
        }
        self.last_layout = None;
    }

    pub fn is_attached(&self, extension: &str) -> bool {
        self.extension.as_deref() == Some(extension) && self.alive()
    }

    pub fn close(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            let _ = unsafe { PostMessageW(hwnd as *mut _, WM_CLOSE, 0, 0) };
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
        self.extension = None;
        self.log_path = None;
        self.spawned_at = None;
        self.last_layout = None;
        self.embedded = false;
    }

    fn alive(&self) -> bool {
        if let Some(hwnd) = self.hwnd {
            return is_window(hwnd);
        }
        self.spawned_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(8))
    }

    /// Place the console over `hole` (egui points). `None` hides it.
    pub fn sync(&mut self, frame: &eframe::Frame, hole: Option<egui::Rect>, ctx: &egui::Context) {
        let Some(parent) = viewport_hwnd(frame) else {
            return;
        };
        if self.hwnd.is_none() {
            if let Some(hwnd) = find_window_by_title(&self.title) {
                self.embedded = embed_child(hwnd, parent);
                if !self.embedded {
                    apply_no_activate(hwnd);
                }
                self.hwnd = Some(hwnd);
            } else if self
                .spawned_at
                .is_some_and(|t| t.elapsed() > Duration::from_secs(12))
            {
                self.child = None;
                self.spawned_at = None;
            }
        }
        let Some(hwnd) = self.hwnd else {
            return;
        };
        if !is_window(hwnd) {
            self.hwnd = None;
            self.child = None;
            self.embedded = false;
            return;
        }
        let Some(rect) = hole.filter(|r| r.width() > 40.0 && r.height() > 40.0) else {
            show_window(hwnd, false);
            return;
        };
        let layout = if self.embedded {
            egui_rect_to_client(rect, ctx.pixels_per_point())
        } else {
            crate::adapters::desktop::window::egui_rect_to_screen(
                parent,
                rect,
                ctx.pixels_per_point(),
            )
        };
        if !layout.visible {
            show_window(hwnd, false);
            return;
        }
        if self.last_layout != Some(layout) {
            set_pos(hwnd, layout);
            self.last_layout = Some(layout);
        } else {
            show_window(hwnd, true);
        }
        // Keep app focus so `~` still reaches egui (console must not steal keys).
        restore_parent_focus(parent, hwnd);
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

fn hide_new_console(title: &str, budget: Duration) -> Option<isize> {
    let started = Instant::now();
    while started.elapsed() < budget {
        if let Some(hwnd) = find_window_by_title(title) {
            show_window(hwnd, false);
            apply_no_activate(hwnd);
            return Some(hwnd);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    None
}

fn restore_parent_focus(parent: isize, console: isize) {
    unsafe {
        let focus = GetFocus();
        if focus.is_null() {
            return;
        }
        if focus == console as *mut _
            || GetAncestor(focus, GA_ROOT) == console as *mut _
            || IsChild(console as *mut _, focus) != 0
        {
            let _ = SetFocus(parent as *mut _);
        }
    }
}

fn apply_no_activate(hwnd: isize) {
    unsafe {
        let ex = GetWindowLongW(hwnd as *mut _, GWL_EXSTYLE) as u32 | WS_EX_NOACTIVATE;
        SetWindowLongW(hwnd as *mut _, GWL_EXSTYLE, ex as i32);
    }
}

fn egui_rect_to_client(rect: egui::Rect, pixels_per_point: f32) -> ScreenLayout {
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    };
    let x = (rect.min.x * ppp).round() as i32;
    let y = (rect.min.y * ppp).round() as i32;
    let w = (rect.width() * ppp).round().max(0.0) as i32;
    let h = (rect.height() * ppp).round().max(0.0) as i32;
    ScreenLayout {
        x,
        y,
        w,
        h,
        visible: w > 1 && h > 1 && rect.is_finite(),
    }
}

fn find_window_by_title(title: &str) -> Option<isize> {
    let mut found = 0isize;
    let mut needle: Vec<u16> = title.encode_utf16().collect();
    needle.push(0);
    unsafe {
        EnumWindows(
            Some(enum_proc),
            &mut EnumCtx {
                needle: needle.as_ptr(),
                found: &mut found,
            } as *mut _ as isize,
        );
    }
    (found != 0).then_some(found)
}

struct EnumCtx {
    needle: *const u16,
    found: *mut isize,
}

unsafe extern "system" fn enum_proc(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
    let ctx = &mut *(lparam as *mut EnumCtx);
    // Include hidden windows: we hide the console at spawn to avoid a flash,
    // then re-find / re-parent it into the terminal hole.
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return 1;
    }
    let mut i = 0;
    while i < n as usize {
        let a = *ctx.needle.add(i);
        if a == 0 {
            break;
        }
        if buf[i] != a {
            return 1;
        }
        i += 1;
    }
    if *ctx.needle.add(i) != 0 {
        return 1;
    }
    *ctx.found = hwnd as isize;
    0
}

fn is_window(hwnd: isize) -> bool {
    unsafe { IsWindow(hwnd as *mut _) != 0 }
}

fn show_window(hwnd: isize, show: bool) {
    unsafe {
        ShowWindow(hwnd as *mut _, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

fn set_pos(hwnd: isize, layout: ScreenLayout) {
    unsafe {
        SetWindowPos(
            hwnd as *mut _,
            HWND_TOP,
            layout.x,
            layout.y,
            layout.w,
            layout.h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Parent the console into the app window and strip chrome so it fills the hole.
/// Returns false if Windows refused (common for modern conhost) — caller floats instead.
fn embed_child(hwnd: isize, parent: isize) -> bool {
    unsafe {
        let style = GetWindowLongW(hwnd as *mut _, GWL_STYLE) as u32;
        let style = (style
            & !(WS_POPUP
                | WS_CAPTION
                | WS_THICKFRAME
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX
                | WS_SYSMENU))
            | WS_CHILD
            | WS_VISIBLE
            | WS_CLIPSIBLINGS;
        SetWindowLongW(hwnd as *mut _, GWL_STYLE, style as i32);
        let ex = GetWindowLongW(hwnd as *mut _, GWL_EXSTYLE) as u32 | WS_EX_NOACTIVATE;
        SetWindowLongW(hwnd as *mut _, GWL_EXSTYLE, ex as i32);
        SetLastError(0);
        let _ = SetParent(hwnd as *mut _, parent as *mut _);
        let ok = GetParent(hwnd as *mut _) == parent as *mut _;
        if !ok {
            let style = (GetWindowLongW(hwnd as *mut _, GWL_STYLE) as u32) & !WS_CHILD;
            SetWindowLongW(hwnd as *mut _, GWL_STYLE, style as i32);
            SetWindowPos(
                hwnd as *mut _,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            return false;
        }
        SetWindowPos(
            hwnd as *mut _,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
        true
    }
}

const WM_CLOSE: u32 = 0x0010;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    #[test]
    fn terminal_reads_existing_utf8_lines_and_follows_appends() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/console-tests/中文 space ' & {stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("run.log");
        std::fs::write(&log, "工作流开始\n").unwrap();
        let mut child = tail_command(&log, "测试终端").unwrap()
            .creation_flags(0x08000000) // CREATE_NO_WINDOW: regression test only
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if tx.send(line).is_err() { break; }
            }
        });
        let first = rx.recv_timeout(Duration::from_secs(10));
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "TTS 第 1 段完成").unwrap();
        file.flush().unwrap();
        let second = rx.recv_timeout(Duration::from_secs(5));
        let _ = child.kill();
        let _ = child.wait();
        reader.join().unwrap();
        let mut error = String::new();
        child.stderr.take().unwrap().read_to_string(&mut error).unwrap();
        assert_eq!(first.unwrap_or_else(|e| panic!("{e}: {error}")).unwrap(), "工作流开始");
        assert_eq!(second.unwrap_or_else(|e| panic!("{e}: {error}")).unwrap(), "TTS 第 1 段完成");
    }
}

const SW_HIDE: i32 = 0;
const SW_SHOWNOACTIVATE: i32 = 4;
const HWND_TOP: *mut std::ffi::c_void = std::ptr::null_mut();
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SWP_FRAMECHANGED: u32 = 0x0020;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const GA_ROOT: u32 = 2;

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(
        cb: Option<unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> i32>,
        lparam: isize,
    ) -> i32;
    fn IsWindow(hwnd: *mut std::ffi::c_void) -> i32;
    fn GetWindowTextW(hwnd: *mut std::ffi::c_void, buf: *mut u16, max: i32) -> i32;
    fn ShowWindow(hwnd: *mut std::ffi::c_void, cmd: i32) -> i32;
    fn SetWindowPos(
        hwnd: *mut std::ffi::c_void,
        after: *mut std::ffi::c_void,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn GetWindowLongW(hwnd: *mut std::ffi::c_void, index: i32) -> i32;
    fn SetWindowLongW(hwnd: *mut std::ffi::c_void, index: i32, value: i32) -> i32;
    fn SetParent(
        hwnd: *mut std::ffi::c_void,
        parent: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn GetParent(hwnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn SetLastError(code: u32);
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
    fn GetFocus() -> *mut std::ffi::c_void;
    fn SetFocus(hwnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn IsChild(parent: *mut std::ffi::c_void, child: *mut std::ffi::c_void) -> i32;
    fn GetAncestor(hwnd: *mut std::ffi::c_void, flags: u32) -> *mut std::ffi::c_void;
}
