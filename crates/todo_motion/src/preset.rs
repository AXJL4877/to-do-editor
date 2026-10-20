//! The preset format: keyframed channels, easing and validation.
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Built-in ids that older documents and extensions name directly.
pub const FADE_OUT: &str = "fadeOut";
pub const DISSOLVE: &str = "dissolve";

/// Longest motion window a preset or clip may ask for, in seconds.
pub const MAX_MOTION_SECS: f64 = 10.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MotionKind {
    /// Plays over the first seconds of a clip and rests at identity.
    Intro,
    /// Plays over the last seconds of a clip, starting from identity.
    Outro,
    /// Plays across the cut into a clip from its same-track predecessor.
    Transition,
}

/// Easing of the segment that ends at a key.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Ease {
    #[default]
    Linear,
    /// Cubic ease-in.
    In,
    /// Cubic ease-out.
    Out,
    /// Cubic ease-in-out.
    InOut,
    /// Smoothstep.
    Smooth,
    /// Overshoots and settles (back-out, s = 1.70158).
    BackOut,
    BounceOut,
    SineInOut,
    /// Keeps the previous value until the key.
    Hold,
}

impl Ease {
    pub fn apply(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Linear => u,
            Self::In => u * u * u,
            Self::Out => 1.0 - (1.0 - u).powi(3),
            Self::InOut => {
                if u < 0.5 {
                    4.0 * u * u * u
                } else {
                    1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
                }
            }
            Self::Smooth => u * u * (3.0 - 2.0 * u),
            Self::BackOut => {
                let s = 1.70158;
                let x = u - 1.0;
                1.0 + (s + 1.0) * x.powi(3) + s * x.powi(2)
            }
            Self::BounceOut => bounce_out(u),
            Self::SineInOut => -((std::f64::consts::PI * u).cos() - 1.0) / 2.0,
            Self::Hold => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MotionKey {
    /// Normalized time in the motion window, 0..=1.
    pub t: f64,
    pub v: f64,
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: Ease,
}

fn is_linear(ease: &Ease) -> bool {
    *ease == Ease::Linear
}

/// Keys sorted by `t`. Before the first key and after the last the curve holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(transparent)]
pub struct MotionCurve(pub Vec<MotionKey>);

impl MotionCurve {
    pub fn sample(&self, u: f64) -> Option<f64> {
        let keys = &self.0;
        let first = keys.first()?;
        if u <= first.t {
            return Some(first.v);
        }
        for pair in keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if u <= b.t {
                let span = b.t - a.t;
                let local = if span > 0.0 { (u - a.t) / span } else { 1.0 };
                return Some(a.v + (b.v - a.v) * b.ease.apply(local));
            }
        }
        keys.last().map(|key| key.v)
    }
}

/// Animated channels of one clip. Opacity and scales multiply the clip's own
/// transform; `x`/`y` add canvas-fraction offsets (the transform's units).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionChannels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_x: Option<MotionCurve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_y: Option<MotionCurve>,
}

impl MotionChannels {
    pub fn sample(&self, u: f64) -> MotionSample {
        let at = |curve: &Option<MotionCurve>, identity: f64| {
            curve
                .as_ref()
                .and_then(|curve| curve.sample(u))
                .filter(|value| value.is_finite())
                .unwrap_or(identity)
        };
        MotionSample {
            opacity: at(&self.opacity, 1.0).clamp(0.0, 1.0),
            dx: at(&self.x, 0.0),
            dy: at(&self.y, 0.0),
            scale: at(&self.scale, 1.0).max(0.001),
            scale_x: at(&self.scale_x, 1.0).max(0.001),
            scale_y: at(&self.scale_y, 1.0).max(0.001),
        }
    }

    fn curves(&self) -> [(&'static str, &Option<MotionCurve>); 6] {
        [
            ("opacity", &self.opacity),
            ("x", &self.x),
            ("y", &self.y),
            ("scale", &self.scale),
            ("scaleX", &self.scale_x),
            ("scaleY", &self.scale_y),
        ]
    }
}

/// One evaluated motion: multiply `opacity`/scales, add `dx`/`dy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSample {
    pub opacity: f64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

impl Default for MotionSample {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }
}

impl MotionSample {
    pub fn is_identity(&self) -> bool {
        (self.opacity - 1.0).abs() < 1e-6
            && self.dx.abs() < 1e-6
            && self.dy.abs() < 1e-6
            && (self.scale - 1.0).abs() < 1e-6
            && (self.scale_x - 1.0).abs() < 1e-6
            && (self.scale_y - 1.0).abs() < 1e-6
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionPreset {
    pub id: String,
    pub name: String,
    pub kind: MotionKind,
    /// Suggested window length in seconds; clips store their own.
    pub duration: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The animated clip; for a transition, the incoming one.
    #[serde(default)]
    pub channels: MotionChannels,
    /// Transition only: the outgoing clip (empty = holds unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outgoing: Option<MotionChannels>,
    /// Transition only: draw the outgoing clip above the incoming one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outgoing_on_top: bool,
}

impl MotionPreset {
    /// The animated clip at window progress `u` (0..=1); for a transition, the
    /// incoming clip.
    pub fn sample(&self, u: f64) -> MotionSample {
        self.channels.sample(u)
    }

    /// A transition's outgoing clip at progress `u`.
    pub fn sample_outgoing(&self, u: f64) -> MotionSample {
        self.outgoing
            .as_ref()
            .map_or_else(MotionSample::default, |channels| channels.sample(u))
    }

    /// Check the preset can play without jumps: every curve is well formed and
    /// the resting end of each animated clip is the identity.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_motion_id(&self.id) {
            return Err(format!("motion id `{}` is invalid", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            return Err(format!("motion `{}` needs a name of 1..=64 chars", self.id));
        }
        if !self.duration.is_finite() || self.duration <= 0.0 || self.duration > MAX_MOTION_SECS {
            return Err(format!("motion `{}` duration is out of range", self.id));
        }
        let check = |channels: &MotionChannels, rest: f64, part: &str| -> Result<(), String> {
            for (name, curve) in channels.curves() {
                let Some(curve) = curve else { continue };
                if curve.0.is_empty() || curve.0.len() > 64 {
                    return Err(format!(
                        "motion `{}` {part}.{name} needs 1..=64 keys",
                        self.id
                    ));
                }
                let mut last = -1.0;
                for key in &curve.0 {
                    if !key.t.is_finite() || !(0.0..=1.0).contains(&key.t) || key.t < last {
                        return Err(format!(
                            "motion `{}` {part}.{name} keys must be sorted in 0..=1",
                            self.id
                        ));
                    }
                    if !key.v.is_finite() || key.v.abs() > 100.0 {
                        return Err(format!("motion `{}` {part}.{name} value", self.id));
                    }
                    last = key.t;
                }
            }
            if !channels.sample(rest).is_identity() {
                return Err(format!(
                    "motion `{}` {part} must rest at identity (t = {rest})",
                    self.id
                ));
            }
            Ok(())
        };
        match self.kind {
            MotionKind::Intro => check(&self.channels, 1.0, "channels")?,
            MotionKind::Outro => check(&self.channels, 0.0, "channels")?,
            MotionKind::Transition => {
                check(&self.channels, 1.0, "channels")?;
                check(
                    self.outgoing.as_ref().unwrap_or(&MotionChannels::default()),
                    0.0,
                    "outgoing",
                )?;
            }
        }
        if self.kind != MotionKind::Transition && (self.outgoing.is_some() || self.outgoing_on_top)
        {
            return Err(format!("motion `{}`: outgoing is for transitions", self.id));
        }
        Ok(())
    }
}

/// `[A-Za-z][A-Za-z0-9_.-]{0,63}`: stable across libraries and file names.
pub fn valid_motion_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Presets every project can use without copying them in.
pub fn builtin_motions() -> &'static [MotionPreset] {
    static BUILTIN: OnceLock<Vec<MotionPreset>> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        let presets: Vec<MotionPreset> =
            serde_json::from_str(include_str!("builtin.json")).expect("builtin motions parse");
        for preset in &presets {
            preset.validate().expect("builtin motion is valid");
        }
        presets
    })
}

pub fn is_builtin_motion(id: &str) -> bool {
    builtin_motions().iter().any(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_unique() {
        let presets = builtin_motions();
        let mut ids: Vec<_> = presets.iter().map(|preset| preset.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), presets.len());
        for kind in [MotionKind::Intro, MotionKind::Outro, MotionKind::Transition] {
            assert!(presets.iter().any(|preset| preset.kind == kind));
        }
    }

    #[test]
    fn curves_hold_outside_keys_and_ease_between() {
        let curve = MotionCurve(vec![
            MotionKey {
                t: 0.2,
                v: 0.0,
                ease: Ease::Linear,
            },
            MotionKey {
                t: 0.6,
                v: 1.0,
                ease: Ease::Smooth,
            },
        ]);
        assert_eq!(curve.sample(0.0), Some(0.0));
        assert_eq!(curve.sample(1.0), Some(1.0));
        assert!((curve.sample(0.4).unwrap() - 0.5).abs() < 1e-9);
    }

    /// The former hard-coded entrance curves, kept as the reference the
    /// built-in presets were ported from.
    #[test]
    fn ported_intros_match_the_former_formulas() {
        let get = |id: &str| builtin_motions().iter().find(|p| p.id == id).unwrap();
        for i in 0..=20 {
            let u = f64::from(i) / 20.0;
            let ease_out = 1.0 - (1.0 - u).powi(3);
            let fade = (2.0 * u).min(1.0);
            let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "u={u}: {a} vs {b}");
            close(get("fade").sample(u).opacity, ease_out);
            close(get("rise").sample(u).dy, 0.06 * (1.0 - ease_out));
            close(get("rise").sample(u).opacity, fade);
            let x = u - 1.0;
            let back = 1.0 + 2.70158 * x.powi(3) + 1.70158 * x.powi(2);
            close(get("pop").sample(u).scale, 0.6 + 0.4 * back);
            close(get("pop").sample(u).opacity, (3.0 * u).min(1.0));
            close(get("slideLeft").sample(u).dx, -0.25 * (1.0 - ease_out));
            close(get("drop").sample(u).dy, -0.15 * (1.0 - bounce_out(u)));
            // Grow starts at 1 % instead of clamping there: within 0.01.
            assert!((get("grow").sample(u).scale - ease_out.max(0.01)).abs() <= 0.01);
            // The former outro: smoothstep of the remaining fraction.
            let left = 1.0 - u;
            close(
                get("fadeOut").sample(u).opacity,
                left * left * (3.0 - 2.0 * left),
            );
        }
    }

    #[test]
    fn invalid_presets_are_rejected() {
        let mut preset = builtin_motions()
            .iter()
            .find(|preset| preset.id == "pop")
            .unwrap()
            .clone();
        preset.channels.opacity = Some(MotionCurve(vec![MotionKey {
            t: 0.0,
            v: 0.5,
            ease: Ease::Linear,
        }]));
        assert!(
            preset.validate().is_err(),
            "intro must rest at full opacity"
        );
        preset.channels.opacity = None;
        preset.id = "9bad".into();
        assert!(preset.validate().is_err());
    }
}
