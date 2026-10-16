use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Physical screen rectangle of the preview slot. The native host places a
/// top-level popup in screen pixels, not client-relative points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub visible: bool,
}

pub fn viewport_hwnd(frame: &eframe::Frame) -> Option<isize> {
    let handle = frame.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win) => Some(win.hwnd.get()),
        _ => None,
    }
}

/// Match the client fill so DWM does not paint a light caption/border fringe.
pub fn apply_dark_frame(hwnd: isize) {
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static DONE: AtomicBool = AtomicBool::new(false);
        if DONE.swap(true, Ordering::SeqCst) {
            return;
        }
        let hwnd = hwnd as *mut std::ffi::c_void;
        let dark: i32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const i32 as *const _,
                std::mem::size_of_val(&dark) as u32,
            );
            // COLORREF 0x00BBGGRR for BG #16181b
            let border: u32 = 0x001b_1816;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &border as *const u32 as *const _,
                std::mem::size_of_val(&border) as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                &border as *const u32 as *const _,
                std::mem::size_of_val(&border) as u32,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
    }
}

pub fn egui_rect_to_screen(hwnd: isize, rect: egui::Rect, pixels_per_point: f32) -> ScreenLayout {
    if !rect.is_finite() {
        return ScreenLayout {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            visible: false,
        };
    }
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    };
    let (origin_x, origin_y) = client_origin(hwnd).unwrap_or((0, 0));
    let x = origin_x + (rect.min.x * ppp).round() as i32;
    let y = origin_y + (rect.min.y * ppp).round() as i32;
    let w = (rect.width() * ppp).round().max(0.0) as i32;
    let h = (rect.height() * ppp).round().max(0.0) as i32;
    let visible = w > 1 && h > 1 && rect.is_finite();
    ScreenLayout {
        x,
        y,
        w,
        h,
        visible,
    }
}

pub fn client_origin(hwnd: isize) -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        let mut point = Point { x: 0, y: 0 };
        let ok = unsafe { ClientToScreen(hwnd as *mut std::ffi::c_void, &mut point) };
        if ok == 0 {
            None
        } else {
            Some((point.x, point.y))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        None
    }
}

#[cfg(windows)]
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[cfg(windows)]
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
#[cfg(windows)]
const DWMWA_BORDER_COLOR: u32 = 34;
#[cfg(windows)]
const DWMWA_CAPTION_COLOR: u32 = 35;

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn ClientToScreen(hwnd: *mut std::ffi::c_void, point: *mut Point) -> i32;
}

#[cfg(windows)]
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: *mut std::ffi::c_void,
        attr: u32,
        value: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_preview_slot_is_hidden_before_pixel_conversion() {
        assert_eq!(
            egui_rect_to_screen(0, egui::Rect::NOTHING, 1.5),
            ScreenLayout {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                visible: false
            }
        );
    }
}
