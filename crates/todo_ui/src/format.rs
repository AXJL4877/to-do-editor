//! Clock text and the editor's vertical split. Tick labels themselves come from the view model.

pub fn clock(sec: f64) -> String {
    if !sec.is_finite() || sec < 0.0 {
        return "0:00".to_owned();
    }
    let total = sec.floor() as i64;
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

/// Toolbar, monitor, timeline heights inside `total`.
pub fn band_heights(total: f32) -> (f32, f32, f32) {
    let toolbar = crate::theme::TOOLBAR_H.min(total.max(0.0));
    let rest = (total - toolbar).max(0.0);
    let timeline = if rest < 280.0 {
        rest * 0.42
    } else {
        (rest * 0.32).clamp(220.0, (rest - 80.0).max(220.0))
    };
    let timeline = timeline.min(rest).max(0.0);
    (toolbar, rest - timeline, timeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_matches_the_product_format() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(65.9), "1:05");
        assert_eq!(clock(-1.0), "0:00");
        assert_eq!(clock(f64::NAN), "0:00");
    }

    #[test]
    fn bands_cover_the_window_without_overlap() {
        let (toolbar, monitor, timeline) = band_heights(800.0);
        assert!((toolbar + monitor + timeline - 800.0).abs() < 0.1);
        assert!(timeline >= 220.0);
        assert!(monitor > 0.0);

        let (toolbar, monitor, timeline) = band_heights(200.0);
        assert!((toolbar + monitor + timeline - 200.0).abs() < 0.1);
        assert!(toolbar <= 56.0);
    }
}
