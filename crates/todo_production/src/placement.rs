//! Progressive placement: turn the items a production has produced so far
//! into one batch of timeline edits (`todo_core::GeneratedEdit`). Pure; the
//! application applies the batch as a single undo step and records the
//! outcome with [`record`].
//!
//! Rules (extension spec §8):
//! - a new key is placed; the same key with the same revision is left alone;
//! - a new revision replaces the clip in place and keeps the user's transform;
//!   the producer's timing is used unless the user moved the clip;
//! - a key that was placed but is gone from the timeline was deleted by the
//!   user: it is not placed again until its revision changes;
//! - a linked item follows the actual position of the clip it links to;
//! - lanes of one role keep their stacking (lane 0 at the bottom).
use crate::items::{Item, ItemRole, ItemSource, ItemTransform};
use crate::production::Placement;
use std::collections::{BTreeMap, BTreeSet};
use todo_core::{
    is_image_media, Anchor, ClipAudio, ClipOrigin, GeneratedClip, GeneratedEdit, GeneratedSource,
    LaneHint, MotionRef, Project, TrackKind, Transform,
};

const EPSILON: f64 = 1e-6;

pub fn track_kind(role: ItemRole) -> TrackKind {
    match role {
        ItemRole::Voice | ItemRole::Music => TrackKind::Audio,
        ItemRole::Sfx => TrackKind::Sfx,
        ItemRole::Visual => TrackKind::Video,
        ItemRole::Text | ItemRole::Subtitle => TrackKind::Text,
    }
}

fn role_name(role: ItemRole) -> &'static str {
    match role {
        ItemRole::Voice => "voice",
        ItemRole::Music => "music",
        ItemRole::Sfx => "sfx",
        ItemRole::Visual => "visual",
        ItemRole::Text => "text",
        ItemRole::Subtitle => "subtitle",
    }
}

pub fn slot(role: ItemRole, lane: u32) -> String {
    format!("{}:{lane}", role_name(role))
}

pub struct PlacementInput<'a> {
    pub project: &'a Project,
    pub production: &'a str,
    pub placement: &'a Placement,
    pub items: &'a [Item],
    /// Repository-relative directory item files are relative to.
    pub workdir: &'a str,
    /// The item set is final: clips of keys no longer produced are removed
    /// (unless the user moved them).
    pub complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlacementPlan {
    /// Repository-relative files that must be registered as media first;
    /// their items are skipped in this plan.
    pub adopt: Vec<String>,
    pub edits: Vec<GeneratedEdit>,
    /// Keys the batch puts on the timeline, with their revision.
    pub placed: Vec<(String, String)>,
    /// Positions and transforms the production itself set (not the user's
    /// kept ones); they tell later batches what the user changed.
    pub positions: Vec<(String, f64)>,
    pub transforms: Vec<(String, [f64; 6])>,
    /// (key, revision) the user deleted.
    pub tombstones: Vec<(String, String)>,
    /// Keys no longer tracked (deleted by the user, or removed by the batch).
    pub forget: Vec<String>,
    /// Tombstones contradicted by a clip that is on the timeline after all.
    pub tombstones_cleared: Vec<(String, String)>,
}

impl PlacementPlan {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
            && self.tombstones.is_empty()
            && self.forget.is_empty()
            && self.placed.is_empty()
    }
}

pub fn origin(production: &str, key: &str, rev: &str) -> ClipOrigin {
    ClipOrigin {
        production: production.into(),
        key: key.into(),
        rev: rev.into(),
    }
}

fn transform(item: Option<ItemTransform>) -> Transform {
    item.map_or_else(Transform::default, |t| Transform {
        x: t.x,
        y: t.y,
        scale: t.scale,
        scale_x: t.scale_x,
        scale_y: t.scale_y,
        opacity: t.opacity,
    })
}

fn pack(t: &Transform) -> [f64; 6] {
    [t.x, t.y, t.scale, t.scale_x, t.scale_y, t.opacity]
}

pub fn media_path(workdir: &str, file: &str) -> String {
    let base = workdir.trim_end_matches('/');
    if base.is_empty() {
        file.to_owned()
    } else {
        format!("{base}/{file}")
    }
}

pub fn plan(input: &PlacementInput) -> PlacementPlan {
    let PlacementInput {
        project,
        production,
        placement,
        items,
        workdir,
        complete,
    } = *input;
    let mut out = PlacementPlan::default();
    // This production's clips and the track each slot currently uses.
    let mut clips: BTreeMap<&str, (&todo_core::Clip, &str)> = BTreeMap::new();
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(o) = clip.origin.as_ref().filter(|o| o.production == production) {
                clips.insert(o.key.as_str(), (clip, track.id.as_str()));
            }
        }
    }
    let by_key: BTreeMap<&str, &Item> = items.iter().map(|i| (i.key.as_str(), i)).collect();
    let mut slot_tracks: BTreeMap<String, String> = BTreeMap::new();
    for (key, (_, track)) in &clips {
        if let Some(item) = by_key.get(key) {
            slot_tracks
                .entry(slot(item.role, item.lane))
                .or_insert_with(|| (*track).to_owned());
        }
    }
    // Lanes in use per kind, for stacking new lanes next to their neighbours.
    let mut lanes: BTreeMap<u8, BTreeSet<(u32, String)>> = BTreeMap::new();
    for item in items {
        lanes
            .entry(track_kind(item.role).section_rank())
            .or_default()
            .insert((item.lane, slot(item.role, item.lane)));
    }
    let moved = |key: &str, clip: &todo_core::Clip| {
        placement
            .positions
            .get(key)
            .is_some_and(|at| (at - clip.start).abs() > EPSILON)
    };
    // Where a link parent actually is, relative to where it was produced.
    let link_delta = |item: &Item| -> f64 {
        let Some(parent) = item.link.as_deref().and_then(|k| by_key.get(k)) else {
            return 0.0;
        };
        clips
            .get(parent.key.as_str())
            .map_or(0.0, |(clip, _)| clip.start - (placement.t0 + parent.start))
    };
    // Slots that get a new clip in this batch (resolved by the core).
    let mut placing: BTreeSet<String> = BTreeSet::new();
    let mut ordered: Vec<&Item> = items.iter().collect();
    // Lower lanes first, so a new upper lane has its anchor in the batch.
    ordered.sort_by(|a, b| {
        let rank = |i: &Item| (track_kind(i.role).section_rank(), i.lane);
        rank(a).cmp(&rank(b)).then(a.start.total_cmp(&b.start))
    });
    for item in ordered {
        let desired = (placement.t0 + item.start + link_delta(item)).max(0.0);
        let source = match &item.source {
            ItemSource::Text { content } => GeneratedSource::Text(content.clone()),
            ItemSource::File { file, r#in } => {
                let path = media_path(workdir, file);
                match project
                    .media
                    .iter()
                    .find(|m| m.path.as_deref() == Some(path.as_str()))
                {
                    Some(media) => GeneratedSource::Media {
                        media_id: media.id.clone(),
                        offset: *r#in,
                    },
                    None => {
                        if !out.adopt.contains(&path) {
                            out.adopt.push(path);
                        }
                        continue;
                    }
                }
            }
        };
        let duration = clamp_duration(project, &source, item.duration);
        let audible = item.role.is_audio();
        let audio = (audible && (item.gain.is_some() || item.fade.is_some())).then(|| ClipAudio {
            gain: item.gain.unwrap_or(1.0),
            fade_in: item.fade.map_or(0.0, |f| f.r#in),
            fade_out: item.fade.map_or(0.0, |f| f.out),
        });
        let outro = (!audible)
            .then(|| {
                item.outro.clone().or_else(|| {
                    item.fade
                        .map(|f| f.out)
                        .filter(|s| *s > 0.0)
                        .map(|s| MotionRef::new(todo_core::motion::FADE_OUT, s))
                })
            })
            .flatten();
        let transition = (!audible).then(|| item.transition.clone()).flatten();
        let produced = transform(item.transform);
        let new_origin = origin(production, &item.key, &item.rev);
        if let Some((clip, _)) = clips.get(item.key.as_str()) {
            let start = if moved(&item.key, clip) {
                clip.start
            } else {
                desired
            };
            let current = clip.origin.as_ref().map(|o| o.rev.as_str());
            if current != Some(item.rev.as_str()) {
                // Keep a transform the user changed; otherwise take the new one.
                let untouched = placement
                    .transforms
                    .get(&item.key)
                    .is_none_or(|t| *t == pack(&clip.transform));
                if untouched {
                    out.transforms.push((item.key.clone(), pack(&produced)));
                }
                if start == desired {
                    out.positions.push((item.key.clone(), desired));
                }
                out.edits.push(GeneratedEdit::Update {
                    clip_id: clip.id.clone(),
                    start,
                    duration,
                    source,
                    transform: untouched.then_some(produced),
                    intro: item.intro.clone(),
                    outro,
                    transition,
                    audio,
                    origin: new_origin,
                });
                out.placed.push((item.key.clone(), item.rev.clone()));
            } else if (clip.start - start).abs() > EPSILON {
                out.edits.push(GeneratedEdit::Move {
                    clip_id: clip.id.clone(),
                    start,
                });
                out.positions.push((item.key.clone(), start));
                out.placed.push((item.key.clone(), item.rev.clone()));
            } else if placement.placed.get(&item.key) != Some(&item.rev) {
                // On the timeline but missing from the record (an older
                // save raced the placement): adopt it as placed, as it is.
                out.placed.push((item.key.clone(), item.rev.clone()));
                out.positions.push((item.key.clone(), clip.start));
                out.transforms
                    .push((item.key.clone(), pack(&clip.transform)));
                out.tombstones_cleared
                    .push((item.key.clone(), item.rev.clone()));
            }
            continue;
        }
        if let Some(placed_rev) = placement.placed.get(&item.key) {
            // Placed earlier and gone now: the user removed it.
            out.tombstones.push((item.key.clone(), placed_rev.clone()));
            out.forget.push(item.key.clone());
            if placed_rev == &item.rev {
                continue;
            }
        }
        if placement
            .tombstones
            .contains(&(item.key.clone(), item.rev.clone()))
        {
            continue;
        }
        let kind = track_kind(item.role);
        let own = slot(item.role, item.lane);
        // The nearest lane under this one that has (or gets) a track, and
        // the nearest one above, as anchors for a new track.
        let neighbours = lanes.get(&kind.section_rank());
        let anchor = |slot: &String| -> Option<Anchor> {
            match slot_tracks.get(slot) {
                Some(track) => Some(Anchor::Track(track.clone())),
                None => placing.contains(slot).then(|| Anchor::Slot(slot.clone())),
            }
        };
        let above = neighbours.and_then(|set| {
            set.iter()
                .filter(|(lane, s)| {
                    (*lane < item.lane || (*lane == item.lane && *s < own)) && *s != own
                })
                .rev()
                .find_map(|(_, s)| anchor(s))
        });
        let below = neighbours.and_then(|set| {
            set.iter()
                .filter(|(lane, s)| *lane > item.lane && *s != own)
                .find_map(|(_, s)| anchor(s))
        });
        placing.insert(own.clone());
        out.positions.push((item.key.clone(), desired));
        out.transforms.push((item.key.clone(), pack(&produced)));
        out.edits.push(GeneratedEdit::Place {
            kind,
            lane: LaneHint {
                preferred: slot_tracks.get(&own).cloned(),
                slot: own,
                above,
                below,
            },
            clip: GeneratedClip {
                start: desired,
                duration,
                source,
                transform: produced,
                intro: item.intro.clone(),
                outro,
                transition,
                audio,
                origin: new_origin,
            },
        });
        out.placed.push((item.key.clone(), item.rev.clone()));
    }
    if complete {
        for (key, (clip, _)) in &clips {
            if !by_key.contains_key(key) && !moved(key, clip) {
                out.edits.push(GeneratedEdit::Remove {
                    clip_id: clip.id.clone(),
                });
                out.forget.push((*key).to_owned());
            }
        }
    }
    // Keys placed earlier, no longer produced and no longer on the timeline.
    for (key, rev) in &placement.placed {
        if !by_key.contains_key(key.as_str()) && !clips.contains_key(key.as_str()) {
            out.tombstones.push((key.clone(), rev.clone()));
            out.forget.push(key.clone());
        }
    }
    out
}

/// Audio and video items cannot run past their media.
fn clamp_duration(project: &Project, source: &GeneratedSource, duration: f64) -> f64 {
    let GeneratedSource::Media { media_id, offset } = source else {
        return duration;
    };
    match project.media.iter().find(|m| &m.id == media_id) {
        Some(media) if !is_image_media(media) && media.duration.is_finite() => {
            duration.min((media.duration - offset).max(0.0))
        }
        _ => duration,
    }
}

/// Record a batch that was applied.
pub fn record(placement: &mut Placement, plan: &PlacementPlan) {
    for key in &plan.forget {
        placement.placed.remove(key);
        placement.positions.remove(key);
        placement.transforms.remove(key);
    }
    for stale in &plan.tombstones_cleared {
        placement.tombstones.remove(stale);
    }
    placement.tombstones.extend(plan.tombstones.iter().cloned());
    for (key, rev) in &plan.placed {
        placement.placed.insert(key.clone(), rev.clone());
    }
    for (key, at) in &plan.positions {
        placement.positions.insert(key.clone(), *at);
    }
    for (key, t) in &plan.transforms {
        placement.transforms.insert(key.clone(), *t);
    }
}
