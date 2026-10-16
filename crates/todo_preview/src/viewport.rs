//! Wire values for the editor camera. Geometry and gestures live in interaction.
use crate::PreviewError;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewViewport {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub show_outside: bool,
}

pub(crate) fn encode_viewport(
    buf: &mut Vec<u8>,
    value: PreviewViewport,
) -> Result<(), PreviewError> {
    if ![
        value.left,
        value.top,
        value.width,
        value.height,
        value.left + value.width,
        value.top + value.height,
    ]
    .iter()
    .all(|v| v.is_finite())
        || value.width <= 0.0
        || value.height <= 0.0
    {
        return Err(PreviewError::Invalid(
            "viewport must be finite with positive dimensions",
        ));
    }
    let _ = writeln!(
        buf,
        "viewport {} {} {} {} {}",
        value.left,
        value.top,
        value.width,
        value.height,
        u8::from(value.show_outside)
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewNavigationKind {
    Wheel,
    PanDown,
    PanMove,
    PanUp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewNavigation {
    pub kind: PreviewNavigationKind,
    pub delta: f64,
    pub x: f64,
    pub y: f64,
}

pub(crate) fn parse_navigation(
    mut parts: std::str::SplitWhitespace<'_>,
) -> Option<PreviewNavigation> {
    let kind = match parts.next()? {
        "wheel" => PreviewNavigationKind::Wheel,
        "pan-down" => PreviewNavigationKind::PanDown,
        "pan-move" => PreviewNavigationKind::PanMove,
        "pan-up" => PreviewNavigationKind::PanUp,
        _ => return None,
    };
    let delta: f64 = parts.next()?.parse().ok()?;
    let x: f64 = parts.next()?.parse().ok()?;
    let y: f64 = parts.next()?.parse().ok()?;
    if !delta.is_finite() || !x.is_finite() || !y.is_finite() || parts.next().is_some() {
        return None;
    }
    Some(PreviewNavigation { kind, delta, x, y })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{parse_host_line, ParsedLine};

    #[test]
    fn viewport_wire_preserves_offscreen_frame_and_rejects_invalid_commands_atomically() {
        let value = PreviewViewport {
            left: -0.25,
            top: 0.125,
            width: 1.5,
            height: 0.75,
            show_outside: true,
        };
        let mut buffer = Vec::new();
        encode_viewport(&mut buffer, value).unwrap();
        assert_eq!(buffer, b"viewport -0.25 0.125 1.5 0.75 1\n");
        let before = buffer.clone();
        for invalid in [
            PreviewViewport {
                width: 0.0,
                ..value
            },
            PreviewViewport {
                left: f64::NAN,
                ..value
            },
            PreviewViewport {
                height: f64::INFINITY,
                ..value
            },
        ] {
            assert!(encode_viewport(&mut buffer, invalid).is_err());
            assert_eq!(buffer, before);
        }
    }

    #[test]
    fn navigation_parses_window_fractions_without_clamping_captured_positions() {
        for (token, kind) in [
            ("wheel", PreviewNavigationKind::Wheel),
            ("pan-down", PreviewNavigationKind::PanDown),
            ("pan-move", PreviewNavigationKind::PanMove),
            ("pan-up", PreviewNavigationKind::PanUp),
        ] {
            let ParsedLine::Navigation(input) =
                parse_host_line(&format!("preview-navigation {token} 0 -0.25 1.5")).unwrap()
            else {
                panic!("navigation expected")
            };
            assert_eq!(input.kind, kind);
            assert_eq!((input.x, input.y), (-0.25, 1.5));
        }
        for invalid in [
            "preview-navigation wheel NaN 0 0",
            "preview-navigation unknown 0 0 0",
            "preview-navigation pan-up 0 inf 0",
            "preview-navigation wheel 1 0 0 extra",
        ] {
            assert!(parse_host_line(invalid).is_none());
        }
    }
}
