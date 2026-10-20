//! Clips produced by AI productions and extensions, applied as one batch so
//! progressive placement is a single undo step. The batch is planned by
//! `todo_production::placement`; here only the timeline rules are enforced.
use super::place_media::unique_id;
use super::text::ensure_style;
use super::track_space::{allocate, free};
use super::tracks::empty_track;
use super::validate_placement;
use crate::model::clip::{normalize_clip, text_clip};
use crate::{
    find_clip, ClipAudio, ClipOrigin, DomainError, Intro, MotionRef, Project, TextContent,
    TrackKind, Transform,
};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedSource {
    /// A library item, starting at source second `offset`.
    Media {
        media_id: String,
        offset: f64,
    },
    Text(TextContent),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedClip {
    pub start: f64,
    pub duration: f64,
    pub source: GeneratedSource,
    pub transform: Transform,
    pub intro: Option<Intro>,
    pub outro: Option<MotionRef>,
    /// Transition from the clip ending where this one starts on its lane.
    pub transition: Option<MotionRef>,
    pub audio: Option<ClipAudio>,
    pub origin: ClipOrigin,
}

/// Where a new clip goes. `slot` names a lane of one production (for
/// example `visual:2`); every clip of a slot shares one track. `preferred` is
/// the slot's current track. A slot without a track gets a new one directly
/// above the track of slot `above` (or below `below`), so the lanes of one
/// production keep their stacking order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaneHint {
    pub slot: String,
    pub preferred: Option<String>,
    pub above: Option<Anchor>,
    pub below: Option<Anchor>,
}

/// A neighbouring lane: an existing track, or a slot placed earlier in the
/// same batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Track(String),
    Slot(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedEdit {
    Place {
        kind: TrackKind,
        lane: LaneHint,
        clip: GeneratedClip,
    },
    /// New content for a clip the production placed earlier. `transform`
    /// replaces the clip's transform; `None` keeps the user's.
    Update {
        clip_id: String,
        start: f64,
        duration: f64,
        source: GeneratedSource,
        transform: Option<Transform>,
        intro: Option<Intro>,
        outro: Option<MotionRef>,
        transition: Option<MotionRef>,
        audio: Option<ClipAudio>,
        origin: ClipOrigin,
    },
    Move {
        clip_id: String,
        start: f64,
    },
    Remove {
        clip_id: String,
    },
    /// Library presets the batch's clips name, copied in first.
    AddMotions(Vec<crate::MotionPreset>),
}

impl GeneratedEdit {
    /// Motion presets the edit names (intro, outro, transition).
    pub fn motions(&self) -> Vec<&MotionRef> {
        let (intro, outro, transition) = match self {
            Self::Place { clip, .. } => (&clip.intro, &clip.outro, &clip.transition),
            Self::Update {
                intro,
                outro,
                transition,
                ..
            } => (intro, outro, transition),
            _ => return Vec::new(),
        };
        [intro, outro, transition].into_iter().flatten().collect()
    }
}

fn build_clip(project: &Project, generated: GeneratedClip) -> crate::Clip {
    let id = unique_id(project);
    let mut clip = match generated.source {
        GeneratedSource::Media { media_id, offset } => normalize_clip(
            id,
            media_id,
            generated.start,
            generated.duration,
            offset,
            1.0,
        ),
        GeneratedSource::Text(content) => {
            text_clip(id, generated.start, generated.duration, content)
        }
    };
    clip.transform = generated.transform;
    clip.intro = generated.intro;
    clip.outro = generated.outro;
    clip.transition = generated.transition;
    clip.audio = generated.audio;
    clip.origin = Some(generated.origin);
    clip
}

fn track_position(project: &Project, id: &str, kind: TrackKind) -> Option<usize> {
    project
        .tracks
        .iter()
        .position(|t| t.id == id && t.kind == kind)
}

/// Pick (or create) the lane for a clip and remember it for its slot.
fn place_lane(
    project: &mut Project,
    slots: &mut HashMap<String, String>,
    kind: TrackKind,
    lane: &LaneHint,
    start: f64,
    duration: f64,
) -> String {
    let fits = |project: &Project, id: &str| {
        project
            .tracks
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.kind == kind && free(t, start, duration))
    };
    let current = slots
        .get(&lane.slot)
        .cloned()
        .or_else(|| lane.preferred.clone())
        .filter(|id| track_position(project, id, kind).is_some());
    if let Some(id) = current {
        if fits(project, &id) {
            slots.insert(lane.slot.clone(), id.clone());
            return id;
        }
        // The slot's lane is taken here (the user put something there): a
        // new lane directly above it keeps the clip in the slot's place in
        // the stack, so it cannot end up under a backdrop.
        let index = track_position(project, &id, kind).expect("checked above");
        let track = empty_track(project, kind, true);
        let new_id = track.id.clone();
        project.tracks.insert(index, track);
        return new_id;
    }
    let anchor = |anchor: &Option<Anchor>| {
        let id = match anchor.as_ref()? {
            Anchor::Track(id) => id,
            Anchor::Slot(slot) => slots.get(slot)?,
        };
        track_position(project, id, kind)
    };
    let id = if let Some(index) = anchor(&lane.above) {
        // Earlier tracks draw on top: directly above = same index.
        let track = empty_track(project, kind, true);
        let id = track.id.clone();
        project.tracks.insert(index, track);
        id
    } else if let Some(index) = anchor(&lane.below) {
        let track = empty_track(project, kind, true);
        let id = track.id.clone();
        project.tracks.insert(index + 1, track);
        id
    } else {
        allocate(project, kind, None, &[(start, duration)])
    };
    slots.insert(lane.slot.clone(), id.clone());
    id
}

fn insert(project: &mut Project, track_id: &str, clip: crate::Clip) -> Result<(), DomainError> {
    let track = project
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .ok_or_else(|| DomainError::TrackNotFound(track_id.into()))?;
    validate_placement(project, track, &clip, &[])?;
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    track.clips.push(clip);
    track.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(())
}

fn take_clip(project: &mut Project, clip_id: &str) -> Result<(String, crate::Clip), DomainError> {
    let found =
        find_clip(project, clip_id).ok_or_else(|| DomainError::ClipNotFound(clip_id.into()))?;
    if found.track.locked {
        return Err(DomainError::TrackLocked(found.track_id));
    }
    let track_id = found.track_id.clone();
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    let index = track.clips.iter().position(|c| c.id == clip_id).unwrap();
    Ok((track_id, track.clips.remove(index)))
}

/// Put an edited clip back on its lane, or on another free lane of the same
/// kind when the new timing collides with a neighbour.
fn reinsert(project: &mut Project, track_id: &str, clip: crate::Clip) -> Result<(), DomainError> {
    let kind = project
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.kind)
        .ok_or_else(|| DomainError::TrackNotFound(track_id.into()))?;
    let target = allocate(
        project,
        kind,
        Some(track_id),
        &[(clip.start, clip.duration)],
    );
    insert(project, &target, clip)
}

/// Removals first, then updates and moves, then new clips; any failure
/// rejects the whole batch.
pub fn apply_generated(
    mut project: Project,
    edits: Vec<GeneratedEdit>,
) -> Result<Project, DomainError> {
    let rank = |edit: &GeneratedEdit| match edit {
        GeneratedEdit::AddMotions(_) => 0,
        GeneratedEdit::Remove { .. } => 0,
        GeneratedEdit::Update { .. } | GeneratedEdit::Move { .. } => 1,
        GeneratedEdit::Place { .. } => 2,
    };
    let mut edits = edits;
    edits.sort_by_key(rank);
    let mut slots = HashMap::new();
    for edit in edits {
        match edit {
            GeneratedEdit::AddMotions(presets) => {
                project = super::motion::add_motions(project, presets)?;
            }
            GeneratedEdit::Remove { clip_id } => {
                take_clip(&mut project, &clip_id)?;
            }
            GeneratedEdit::Move { clip_id, start } => {
                let (track_id, mut clip) = take_clip(&mut project, &clip_id)?;
                clip.start = start;
                reinsert(&mut project, &track_id, clip)?;
            }
            GeneratedEdit::Update {
                clip_id,
                start,
                duration,
                source,
                transform,
                intro,
                outro,
                transition,
                audio,
                origin,
            } => {
                if let GeneratedSource::Text(content) = &source {
                    project = ensure_style(project, &content.style_id)?;
                }
                let (track_id, old) = take_clip(&mut project, &clip_id)?;
                let mut clip = build_clip(
                    &project,
                    GeneratedClip {
                        start,
                        duration,
                        source,
                        transform: transform.unwrap_or(old.transform),
                        intro,
                        outro,
                        transition,
                        audio,
                        origin,
                    },
                );
                clip.id = old.id;
                clip.link_id = old.link_id;
                reinsert(&mut project, &track_id, clip)?;
            }
            GeneratedEdit::Place { kind, lane, clip } => {
                if let GeneratedSource::Text(content) = &clip.source {
                    project = ensure_style(project, &content.style_id)?;
                }
                let clip = build_clip(&project, clip);
                let track_id = place_lane(
                    &mut project,
                    &mut slots,
                    kind,
                    &lane,
                    clip.start,
                    clip.duration,
                );
                insert(&mut project, &track_id, clip)?;
            }
        }
    }
    Ok(project)
}
