//! Keep restored window bounds when eframe saves during minimization/maximization.

use egui::{Pos2, Vec2, ViewportInfo};
use serde::{Deserialize, Serialize};

/// Matches the `window` storage record consumed by egui-winit/eframe 0.31.1.
/// Positions are physical pixels; inner size is in egui points. Keep this adapter
/// in sync when upgrading eframe. Missing fields retain application startup defaults;
/// unrecognized fields are ignored by serde, as they are by the pinned upstream type.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct WindowState {
    inner_position_pixels: Option<Pos2>,
    outer_position_pixels: Option<Pos2>,
    fullscreen: bool,
    maximized: bool,
    inner_size_points: Option<Vec2>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            inner_position_pixels: None,
            outer_position_pixels: None,
            fullscreen: false,
            maximized: false,
            inner_size_points: Some(egui::vec2(1440.0, 900.0)),
        }
    }
}

impl WindowState {
    pub(super) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, "window"))
            .unwrap_or_default()
    }

    pub(super) fn observe(&mut self, viewport: &ViewportInfo, pixels_per_point: f32) {
        if viewport.minimized == Some(true)
            || !pixels_per_point.is_finite()
            || pixels_per_point <= 0.0
        {
            return;
        }
        let Some(inner) = viewport
            .inner_rect
            .filter(|rect| rect.is_finite() && rect.is_positive())
        else {
            return;
        };
        self.maximized = viewport.maximized.unwrap_or(self.maximized);
        self.fullscreen = viewport.fullscreen.unwrap_or(self.fullscreen);
        if self.maximized || self.fullscreen {
            return;
        }
        self.inner_size_points = Some(inner.size());
        // egui-winit divides native monitor coordinates by this same factor
        // (context zoom * native scale) before populating ViewportInfo.
        self.inner_position_pixels = Some(inner.min * pixels_per_point);
        self.outer_position_pixels = viewport
            .outer_rect
            .filter(|rect| rect.is_finite() && rect.is_positive())
            .map(|rect| rect.min * pixels_per_point);
    }

    pub(super) fn save(&self, storage: &mut dyn eframe::Storage) {
        // eframe saves its native record before App::save; replace transient
        // native bounds with the last usable restored bounds tracked above.
        eframe::set_value(storage, "window", self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);

    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn flush(&mut self) {}
    }

    fn visible(size: Vec2) -> ViewportInfo {
        ViewportInfo {
            inner_rect: Some(egui::Rect::from_min_size(egui::pos2(100.0, 80.0), size)),
            outer_rect: Some(egui::Rect::from_min_size(egui::pos2(96.0, 60.0), size)),
            minimized: Some(false),
            maximized: Some(false),
            fullscreen: Some(false),
            ..Default::default()
        }
    }

    #[test]
    fn saved_window_bounds_restore_in_eframe_with_physical_positions_and_point_size() {
        let mut state = WindowState::default();
        state.observe(&visible(egui::vec2(1200.0, 800.0)), 2.0);
        let mut storage = Storage::default();
        state.save(&mut storage);
        let upstream: egui_winit::WindowSettings = eframe::get_value(&storage, "window").unwrap();
        assert_eq!(
            upstream.inner_size_points(),
            Some(egui::vec2(1200.0, 800.0))
        );
        let restored = WindowState::load(Some(&storage));
        assert_eq!(
            restored.inner_position_pixels,
            Some(egui::pos2(200.0, 160.0))
        );
        assert_eq!(
            restored.outer_position_pixels,
            Some(egui::pos2(192.0, 120.0))
        );
    }

    #[test]
    fn maximized_and_minimized_shutdown_preserves_normal_restore_bounds() {
        let mut state = WindowState::default();
        state.observe(&visible(egui::vec2(1200.0, 800.0)), 1.0);
        let mut maximized = visible(egui::vec2(1920.0, 1080.0));
        maximized.maximized = Some(true);
        state.observe(&maximized, 1.0);
        let mut minimized = visible(Vec2::ZERO);
        minimized.minimized = Some(true);
        state.observe(&minimized, 1.0);
        let mut storage = Storage::default();
        state.save(&mut storage);
        let reopened = WindowState::load(Some(&storage));
        assert_eq!(reopened.inner_size_points, Some(egui::vec2(1200.0, 800.0)));
        assert!(reopened.maximized);
        assert_eq!(reopened.outer_position_pixels, Some(egui::pos2(96.0, 60.0)));
    }

    #[test]
    fn zero_and_invalid_bounds_do_not_replace_the_last_visible_size() {
        let mut state = WindowState::default();
        state.observe(&visible(egui::vec2(1200.0, 800.0)), 1.0);
        for size in [Vec2::ZERO, egui::vec2(f32::NAN, 800.0)] {
            state.observe(&visible(size), 1.0);
        }
        state.observe(&visible(egui::vec2(640.0, 480.0)), 0.0);
        assert_eq!(state.inner_size_points, Some(egui::vec2(1200.0, 800.0)));
    }
}
