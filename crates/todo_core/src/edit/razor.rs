use crate::model::clip::{normalize_existing, Clip, ClipSource, MIN_CLIP_DURATION};
use crate::model::text::TextAnimation;
use crate::model::Project;

use super::mutate::map_tracks;

pub(super) fn minimum_piece(fps: f64) -> f64 {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    MIN_CLIP_DURATION.max(1.0 / fps)
}

/// Split at timeline `time`. The right piece of a media clip continues the source;
/// the right piece of a text clip keeps the content but not its intro, so a cut
/// never replays the entrance. Entrance and incoming transition stay on the
/// left piece, the exit moves to the right one: the cut between them is straight.
pub(super) fn split_clip(
    clip: &Clip,
    time: f64,
    new_id: String,
    minimum: f64,
) -> Option<(Clip, Clip)> {
    let local = time - clip.start;
    if !time.is_finite() || local + 1e-9 < minimum || clip.duration - local + 1e-9 < minimum {
        return None;
    }
    let mut left = clip.clone();
    left.duration = local;
    left.outro = None;
    let mut right = clip.clone();
    right.intro = None;
    right.transition = None;
    right.id = new_id;
    right.start = time;
    right.duration = clip.duration - local;
    match &mut right.source {
        ClipSource::Media(source) => source.offset += local * source.rate,
        ClipSource::Text(content) => content.animation = TextAnimation::default(),
    }
    Some((normalize_existing(left), normalize_existing(right)))
}

/// Whether `right` is the piece [`split_clip`] cut from `original`, next to `left`.
pub fn is_split_piece(original: &Clip, left: &Clip, right: &Clip) -> bool {
    if (right.start - (left.start + left.duration)).abs() >= 1e-7 {
        return false;
    }
    match (&original.source, &right.source) {
        (ClipSource::Media(old), ClipSource::Media(piece)) => {
            piece.media_id == old.media_id
                && (piece.offset - (old.offset + left.duration * old.rate)).abs() < 1e-7
        }
        (ClipSource::Text(old), ClipSource::Text(piece)) => {
            piece.text == old.text && piece.style_id == old.style_id && piece.layout == old.layout
        }
        _ => false,
    }
}

pub fn razor<F>(project: Project, time: f64, track_id: Option<&str>, mut new_id: F) -> Project
where
    F: FnMut() -> String,
{
    let minimum = minimum_piece(project.fps);
    let mut used: std::collections::HashSet<String> = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
        .collect();
    map_tracks(project, |track| {
        if track.locked || track_id.is_some_and(|id| id != track.id) {
            return track;
        }
        let mut next_clips = Vec::new();
        for clip in track.clips {
            let hit = time > clip.start && time < clip.start + clip.duration;
            if !hit {
                next_clips.push(clip);
                continue;
            }
            let proposed = new_id();
            let mut id = proposed;
            while used.contains(&id) {
                id = default_clip_id();
            }
            used.insert(id.clone());
            match split_clip(&clip, time, id, minimum) {
                Some((left, right)) => {
                    next_clips.push(left);
                    next_clips.push(right);
                }
                None => next_clips.push(clip),
            }
        }
        crate::model::track::Track {
            clips: next_clips,
            ..track
        }
    })
}

pub fn default_clip_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("c_{n:x}")
}
