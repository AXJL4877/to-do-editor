//! Reveal layers at a moment: how far the atlas has played, and where the
//! chalk hand is. Pure geometry shared by preview and export.
use crate::{MediaItem, RevealHand, Transform};

/// Hand stays this long after the last stroke, fading out.
pub const HAND_LINGER: f64 = 0.15;

/// Atlas playback at `media_time`: progress 0..1 and the soft edge as a
/// fraction of the duration (what the reveal shader takes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RevealFrame {
    pub progress: f64,
    pub soft: f64,
}

pub fn reveal_frame(media: &MediaItem, media_time: f64) -> Option<RevealFrame> {
    let info = media.reveal.as_ref()?;
    let duration = media.duration;
    if !(duration.is_finite() && duration > 0.0) {
        return None;
    }
    Some(RevealFrame {
        progress: (media_time / duration).clamp(0.0, 1.0),
        soft: (info.soft_max / duration).clamp(0.0, 1.0),
    })
}

/// The chalk hand as one more picture layer.
#[derive(Debug, Clone, PartialEq)]
pub struct HandLayer {
    /// Repository-relative path of the hand image.
    pub path: String,
    pub size: (u32, u32),
    pub transform: Transform,
}

fn tip_at(path: &[[f64; 3]], time: f64) -> Option<(f64, f64, f64)> {
    let first = path.first()?;
    let last = path.last()?;
    if time < first[0] || time > last[0] + HAND_LINGER {
        return None;
    }
    let fade = if time > last[0] {
        1.0 - (time - last[0]) / HAND_LINGER
    } else {
        1.0
    };
    let i = path.partition_point(|p| p[0] <= time);
    let (x, y) = if i == 0 {
        (first[1], first[2])
    } else if i >= path.len() {
        (last[1], last[2])
    } else {
        let (a, b) = (path[i - 1], path[i]);
        let span = (b[0] - a[0]).max(1e-9);
        let u = ((time - a[0]) / span).clamp(0.0, 1.0);
        (a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u)
    };
    Some((x, y, fade.clamp(0.0, 1.0)))
}

fn sibling(media_path: &str, file: &str) -> String {
    match media_path.rfind('/') {
        Some(slash) => format!("{}/{file}", &media_path[..slash]),
        None => file.to_owned(),
    }
}

/// Where the hand is drawn over a reveal clip shown with `element` (its
/// transform after intro and outro), on a `canvas` of output pixels.
pub fn reveal_hand(
    media: &MediaItem,
    element: &Transform,
    canvas: (u32, u32),
    media_time: f64,
) -> Option<HandLayer> {
    let hand: &RevealHand = media.reveal.as_ref()?.hand.as_ref()?;
    let (u, v, fade) = tip_at(&hand.path, media_time)?;
    let (w, h) = (f64::from(canvas.0), f64::from(canvas.1));
    let (ew, eh) = (f64::from(media.width?), f64::from(media.height?));
    let (sw, sh) = (f64::from(hand.width), f64::from(hand.height));
    if [w, h, ew, eh, sw, sh].iter().any(|v| *v <= 0.0) {
        return None;
    }
    // The element as displayed: centered aspect fit, then its transform.
    let fit = (w / ew).min(h / eh);
    let dw = ew * fit * element.scale * element.scale_x;
    let dh = eh * fit * element.scale * element.scale_y;
    let cx = w * (0.5 + element.x);
    let cy = h * (0.5 + element.y);
    let tip_x = cx + (u - 0.5) * dw;
    let tip_y = cy + (v - 0.5) * dh;
    // The hand keeps its size relative to the element it writes.
    let hand_h = hand.scale * dh.abs();
    let hand_w = hand_h * sw / sh;
    let center_x = tip_x + (0.5 - hand.anchor[0]) * hand_w;
    let center_y = tip_y + (0.5 - hand.anchor[1]) * hand_h;
    let hand_fit = (w / sw).min(h / sh);
    let transform = Transform {
        x: center_x / w - 0.5,
        y: center_y / h - 0.5,
        scale: hand_h / (sh * hand_fit),
        scale_x: 1.0,
        scale_y: 1.0,
        opacity: (element.opacity * fade).clamp(0.0, 1.0),
    };
    let media_path = media.path.as_deref()?;
    Some(HandLayer {
        path: sibling(media_path, &hand.file),
        size: (hand.width, hand.height),
        transform,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlphaPacking, MediaKind, RevealInfo};

    fn atlas() -> MediaItem {
        MediaItem {
            id: "r".into(),
            name: "e.reveal.png".into(),
            path: Some("data/p/work/out/b1/e.reveal.png".into()),
            duration: 2.0,
            has_audio: Some(false),
            width: Some(400),
            height: Some(200),
            kind: Some(MediaKind::Image),
            alpha: Some(AlphaPacking::Reveal),
            reveal: Some(RevealInfo {
                soft_max: 0.5,
                hand: Some(RevealHand {
                    file: "hand_chalk.png".into(),
                    width: 100,
                    height: 200,
                    anchor: [0.0, 0.0],
                    scale: 1.0,
                    path: vec![[0.5, 0.0, 0.0], [1.5, 1.0, 1.0]],
                }),
            }),
            folder_id: None,
        }
    }

    #[test]
    fn progress_and_soft_are_fractions_of_the_duration() {
        let frame = reveal_frame(&atlas(), 1.0).unwrap();
        assert_eq!((frame.progress, frame.soft), (0.5, 0.25));
        assert_eq!(reveal_frame(&atlas(), 9.0).unwrap().progress, 1.0);
    }

    #[test]
    fn the_hand_tip_follows_the_path_and_leaves_after_the_last_stroke() {
        // A 400×200 element fills a 400×200 canvas exactly.
        let element = Transform::default();
        assert!(reveal_hand(&atlas(), &element, (400, 200), 0.2).is_none());
        let mid = reveal_hand(&atlas(), &element, (400, 200), 1.0).unwrap();
        assert_eq!(mid.path, "data/p/work/out/b1/hand_chalk.png");
        // Tip at the element center; anchor (0,0) puts the hand's top-left there.
        // Hand is 200 px high (scale 1 × element height), 100 px wide.
        let (cx, cy) = (
            (mid.transform.x + 0.5) * 400.0,
            (mid.transform.y + 0.5) * 200.0,
        );
        assert!(
            (cx - 250.0).abs() < 1e-6 && (cy - 200.0).abs() < 1e-6,
            "{cx} {cy}"
        );
        assert!((mid.transform.scale - 1.0).abs() < 1e-9);
        let leaving = reveal_hand(&atlas(), &element, (400, 200), 1.6).unwrap();
        assert!(leaving.transform.opacity < 0.5);
        assert!(reveal_hand(&atlas(), &element, (400, 200), 1.7).is_none());
    }
}
