use serde::{Deserialize, Serialize};

/// A clip's optional entrance animation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Intro {
    pub kind: IntroKind,
    pub duration: f64,
}

/// Built-in entrance curves shared by core, extensions and the host.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum IntroKind {
    Fade,
    Rise,
    Pop,
    SlideLeft,
    SlideRight,
    Drop,
    Stamp,
    Grow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntroSample {
    pub opacity: f64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
}

impl Default for IntroSample {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
        }
    }
}

/// Per-clip audio controls. Values are normalized at construction and checked
/// again when resolving a malformed persisted project.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipAudio {
    pub gain: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl Default for ClipAudio {
    fn default() -> Self {
        Self {
            gain: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClipOrigin {
    pub production: String,
    pub key: String,
    pub rev: String,
}

/// Evaluate an intro at seconds from the beginning of a clip.
///
/// Values after the intro are exactly the identity sample, which makes seeking
/// and export use the same deterministic function.
pub fn intro_at(intro: &Intro, time: f64) -> IntroSample {
    if !intro.duration.is_finite() || intro.duration <= 0.0 || !time.is_finite() {
        return IntroSample::default();
    }
    let u = (time / intro.duration).clamp(0.0, 1.0);
    if u >= 1.0 {
        return IntroSample::default();
    }
    let ease_out = 1.0 - (1.0 - u).powi(3);
    let fade = (2.0 * u).min(1.0);
    let mut sample = IntroSample {
        opacity: 1.0,
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
    };
    match intro.kind {
        IntroKind::Fade => sample.opacity = ease_out,
        IntroKind::Rise => {
            sample.dy = 0.06 * (1.0 - ease_out);
            sample.opacity = fade;
        }
        IntroKind::Pop => {
            let s = 1.70158;
            let x = u - 1.0;
            let back_out = 1.0 + (s + 1.0) * x.powi(3) + s * x.powi(2);
            sample.scale = 0.6 + 0.4 * back_out;
            sample.opacity = (3.0 * u).min(1.0);
        }
        IntroKind::SlideLeft => {
            sample.dx = -0.25 * (1.0 - ease_out);
            sample.opacity = fade;
        }
        IntroKind::SlideRight => {
            sample.dx = 0.25 * (1.0 - ease_out);
            sample.opacity = fade;
        }
        IntroKind::Drop => {
            let bounce = bounce_out(u);
            sample.dy = -0.15 * (1.0 - bounce);
            sample.opacity = (4.0 * u).min(1.0);
        }
        IntroKind::Stamp => {
            if u < 0.7 {
                let v = (u / 0.7).clamp(0.0, 1.0);
                sample.scale = 1.3 - 0.3 * v.powi(3);
            } else {
                sample.scale = 1.0 - 0.03 * (std::f64::consts::PI * (u - 0.7) / 0.3).sin();
            }
            sample.opacity = (4.0 * u).min(1.0);
        }
        IntroKind::Grow => sample.scale = ease_out.max(0.01),
    }
    sample
}

/// The transform a picture clip is drawn with at `local_time` seconds from its
/// start: its own transform with the intro curve and the outro fade applied.
/// Preview and export both use this, so they cannot drift apart.
pub fn picture_transform(clip: &crate::Clip, local_time: f64) -> crate::Transform {
    let mut transform = clip.transform;
    if let Some(intro) = clip.intro {
        let sample = intro_at(&intro, local_time);
        transform.x += sample.dx;
        transform.y += sample.dy;
        transform.scale *= sample.scale;
        transform.opacity *= sample.opacity;
    }
    if let Some(outro) = clip.outro.filter(|s| s.is_finite() && *s > 0.0) {
        let left = (clip.duration - local_time).max(0.0);
        if left < outro {
            let u = left / outro;
            // Ease-in on the way out mirrors the intro's ease-out.
            transform.opacity *= u * u * (3.0 - 2.0 * u);
        }
    }
    transform
}

fn bounce_out(mut u: f64) -> f64 {
    const N1: f64 = 7.5625;
    const D1: f64 = 2.75;
    if u < 1.0 / D1 {
        N1 * u * u
    } else if u < 2.0 / D1 {
        u -= 1.5 / D1;
        N1 * u * u + 0.75
    } else if u < 2.5 / D1 {
        u -= 2.25 / D1;
        N1 * u * u + 0.9375
    } else {
        u -= 2.625 / D1;
        N1 * u * u + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_identity_after_duration() {
        for kind in [
            IntroKind::Fade,
            IntroKind::Rise,
            IntroKind::Pop,
            IntroKind::SlideLeft,
            IntroKind::SlideRight,
            IntroKind::Drop,
            IntroKind::Stamp,
            IntroKind::Grow,
        ] {
            assert_eq!(
                intro_at(
                    &Intro {
                        kind,
                        duration: 1.0
                    },
                    1.0
                ),
                IntroSample::default()
            );
            assert_eq!(
                intro_at(
                    &Intro {
                        kind,
                        duration: 1.0
                    },
                    2.0
                ),
                IntroSample::default()
            );
        }
    }

    #[test]
    fn representative_curve_values_are_finite_and_visible() {
        for kind in [
            IntroKind::Fade,
            IntroKind::Rise,
            IntroKind::Pop,
            IntroKind::SlideLeft,
            IntroKind::SlideRight,
            IntroKind::Drop,
            IntroKind::Stamp,
            IntroKind::Grow,
        ] {
            let sample = intro_at(
                &Intro {
                    kind,
                    duration: 2.0,
                },
                1.0,
            );
            assert!(sample.opacity.is_finite() && sample.scale.is_finite());
            assert!(sample.scale > 0.0 && (0.0..=1.0).contains(&sample.opacity));
        }
    }
}
