use serde::{Deserialize, Deserializer, Serialize};
use todo_motion::{MotionKind, MotionPreset, MotionSample, FADE_OUT};

use super::Project;
use crate::Transform;

/// A clip's use of a motion preset: which one (`kind`, a preset id) and how
/// long its window is. Entrances, exits and transitions share this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MotionRef {
    pub kind: String,
    pub duration: f64,
}

impl MotionRef {
    pub fn new(kind: impl Into<String>, duration: f64) -> Self {
        Self {
            kind: kind.into(),
            duration,
        }
    }

    fn window(&self) -> Option<f64> {
        (self.duration.is_finite() && self.duration > 0.0).then_some(self.duration)
    }
}

/// Entrance of a clip (`kind` names an intro preset such as `pop`).
pub type Intro = MotionRef;

/// Persisted outros were a bare number of fade seconds; read both shapes.
pub(crate) fn deserialize_outro<'de, D>(deserializer: D) -> Result<Option<MotionRef>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Seconds(f64),
        Motion(MotionRef),
    }
    Ok(match Option::<Wire>::deserialize(deserializer)? {
        None => None,
        Some(Wire::Seconds(seconds)) => Some(MotionRef::new(FADE_OUT, seconds)),
        Some(Wire::Motion(motion)) => Some(motion),
    }
    .filter(|motion| motion.window().is_some()))
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

/// The preset a project resolves `id` to: its own copy first, then the
/// built-ins. Unknown ids play as no motion.
pub fn find_motion<'a>(project: &'a Project, id: &str) -> Option<&'a MotionPreset> {
    project
        .motions
        .iter()
        .find(|preset| preset.id == id)
        .or_else(|| {
            todo_motion::builtin_motions()
                .iter()
                .find(|preset| preset.id == id)
        })
}

fn find_kind<'a>(project: &'a Project, id: &str, kind: MotionKind) -> Option<&'a MotionPreset> {
    find_motion(project, id).filter(|preset| preset.kind == kind)
}

/// Entrance sample `time` seconds after the clip became visible.
pub fn intro_sample(project: &Project, intro: &MotionRef, time: f64) -> MotionSample {
    let (Some(window), Some(preset)) = (
        intro.window(),
        find_kind(project, &intro.kind, MotionKind::Intro),
    ) else {
        return MotionSample::default();
    };
    if !time.is_finite() || time >= window {
        return MotionSample::default();
    }
    preset.sample((time / window).clamp(0.0, 1.0))
}

/// Exit sample with `left` seconds of visibility remaining.
pub fn outro_sample(project: &Project, outro: &MotionRef, left: f64) -> MotionSample {
    let (Some(window), Some(preset)) = (
        outro.window(),
        find_kind(project, &outro.kind, MotionKind::Outro),
    ) else {
        return MotionSample::default();
    };
    if !left.is_finite() || left >= window {
        return MotionSample::default();
    }
    preset.sample((1.0 - left / window).clamp(0.0, 1.0))
}

/// Multiply opacity and scales, add offsets.
pub fn apply_motion(transform: &mut Transform, sample: MotionSample) {
    transform.x += sample.dx;
    transform.y += sample.dy;
    transform.scale *= sample.scale;
    transform.scale_x *= sample.scale_x;
    transform.scale_y *= sample.scale_y;
    transform.opacity = (transform.opacity * sample.opacity).clamp(0.0, 1.0);
}

/// Extra visibility a transition gives a clip around its own span: `lead`
/// seconds before its start and `tail` after its end.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VisibleSpan {
    pub lead: f64,
    pub tail: f64,
}

/// The transform a picture clip is drawn with at `local_time` seconds from its
/// start: its own transform with the intro and outro applied. Their clocks run
/// on the visible span, so a clip entering through a transition starts its
/// entrance as soon as it appears. Preview, export and `describe` use this.
pub fn picture_transform(
    project: &Project,
    clip: &crate::Clip,
    local_time: f64,
    span: VisibleSpan,
) -> Transform {
    let mut transform = clip.transform;
    if let Some(intro) = &clip.intro {
        apply_motion(
            &mut transform,
            intro_sample(project, intro, local_time + span.lead),
        );
    }
    if let Some(outro) = &clip.outro {
        let left = (clip.duration + span.tail - local_time).max(0.0);
        apply_motion(&mut transform, outro_sample(project, outro, left));
    }
    transform
}
