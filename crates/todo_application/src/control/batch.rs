use super::*;
use crate::{history::same_content, EditCommand, Project, TrimEdge};
use std::collections::{HashMap, HashSet};
use todo_core::{execute_edit, find_clip, is_split_piece, linked_clip_ids};

pub(crate) fn run_batch(
    original: &Project,
    edits: &[ControlEdit],
    dry_run: bool,
) -> Result<(Project, EditReceipt), ControlError> {
    if edits.is_empty() || edits.len() > 128 {
        return Err(ControlError::new(
            "invalid_request",
            "An edit batch must contain 1..128 steps",
        ));
    }
    let mut project = original.clone();
    let mut steps = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        let run = || {
            let command = command(&project, &steps, edit)?;
            let next = execute_edit(project.clone(), command).map_err(ControlError::domain)?;
            let receipt = summarize(&project, &next, matches!(edit, ControlEdit::Split { .. }));
            Ok((next, receipt))
        };
        let (next, receipt) = run().map_err(|mut error: ControlError| {
            error.step = Some(index);
            error
        })?;
        project = next;
        steps.push(receipt);
    }
    let changed = !same_content(original, &project);
    Ok((
        project,
        EditReceipt {
            changed,
            dry_run,
            steps,
        },
    ))
}

fn resolve(
    project: &Project,
    steps: &[EditStepReceipt],
    refs: &[ClipRef],
) -> Result<Vec<String>, ControlError> {
    let mut ids = Vec::new();
    for reference in refs {
        let resolved = match reference {
            ClipRef::Id { id } => vec![id.clone()],
            ClipRef::Created { step } => {
                let Some(receipt) = steps.get(*step) else {
                    return Err(ControlError::new(
                        "invalid_clip_reference",
                        "Created references must name an earlier batch step",
                    ));
                };
                if receipt.created_clips.is_empty() {
                    return Err(ControlError::new(
                        "invalid_clip_reference",
                        "The referenced step created no clips",
                    ));
                }
                receipt
                    .created_clips
                    .iter()
                    .map(|item| item.clip.id.clone())
                    .collect()
            }
        };
        for id in resolved {
            if find_clip(project, &id).is_none() {
                return Err(ControlError::domain(todo_core::DomainError::ClipNotFound(
                    id,
                )));
            }
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        return Err(ControlError::new(
            "invalid_clip_reference",
            "At least one clip is required",
        ));
    }
    Ok(ids)
}

fn primary(
    project: &Project,
    steps: &[EditStepReceipt],
    reference: &ClipRef,
) -> Result<String, ControlError> {
    let ids = resolve(project, steps, std::slice::from_ref(reference))?;
    // Created placement groups list the visual clip first in track order.
    Ok(ids[0].clone())
}

fn command(
    project: &Project,
    steps: &[EditStepReceipt],
    edit: &ControlEdit,
) -> Result<EditCommand, ControlError> {
    Ok(match edit {
        ControlEdit::PlaceMedia {
            media_id,
            track_id,
            timeline_start,
            source_range,
        } => match source_range {
            Some(range) => EditCommand::PlaceMediaRange {
                media_id: media_id.clone(),
                track_id: track_id.clone(),
                time: *timeline_start,
                source_in: range.source_in,
                source_out: range.source_out,
            },
            None => EditCommand::PlaceMedia {
                media_id: media_id.clone(),
                track_id: track_id.clone(),
                time: *timeline_start,
            },
        },
        ControlEdit::MoveClips {
            clips,
            delta,
            track_id,
            linked,
        } => {
            let ids = resolve(project, steps, clips)?;
            EditCommand::MoveClips {
                clip_ids: if *linked {
                    linked_clip_ids(project, &ids)
                } else {
                    ids
                },
                delta: *delta,
                track_id: track_id.clone(),
            }
        }
        ControlEdit::TrimClip {
            clip,
            edge,
            timeline_time,
            linked,
        } => EditCommand::TrimClips {
            primary_id: primary(project, steps, clip)?,
            edge: match edge {
                ClipEdge::In => TrimEdge::In,
                ClipEdge::Out => TrimEdge::Out,
            },
            time: *timeline_time,
            linked: *linked,
        },
        ControlEdit::Split {
            clips,
            timeline_time,
        } => EditCommand::SplitLinked {
            clip_ids: resolve(project, steps, clips)?,
            time: *timeline_time,
        },
        ControlEdit::DeleteClips {
            clips,
            ripple,
            linked,
        } => {
            let ids = resolve(project, steps, clips)?;
            EditCommand::DeleteClips {
                clip_ids: if *linked {
                    linked_clip_ids(project, &ids)
                } else {
                    ids
                },
                ripple: *ripple,
            }
        }
        ControlEdit::AddTrack { kind } => EditCommand::AddTrack { kind: *kind },
        ControlEdit::SetTransform { clip, transform } => EditCommand::SetTransform {
            clip_id: primary(project, steps, clip)?,
            transform: *transform,
        },
        ControlEdit::AddText {
            track_id,
            timeline_start,
            duration,
            content,
        } => EditCommand::AddText {
            track_id: track_id.clone(),
            time: *timeline_start,
            duration: *duration,
            content: content.clone(),
        },
        ControlEdit::SetText { clip, content } => EditCommand::SetText {
            clip_id: primary(project, steps, clip)?,
            content: content.clone(),
        },
        ControlEdit::CreateFolder {
            id,
            name,
            parent_id,
        } => EditCommand::CreateFolder {
            id: id.clone(),
            name: name.clone(),
            parent_id: parent_id.clone(),
        },
        ControlEdit::RenameFolder { id, name } => EditCommand::RenameFolder {
            id: id.clone(),
            name: name.clone(),
        },
        ControlEdit::DeleteFolder { id } => EditCommand::DeleteFolder { id: id.clone() },
        ControlEdit::MoveFolder { id, parent_id } => EditCommand::MoveFolder {
            id: id.clone(),
            parent_id: parent_id.clone(),
        },
        ControlEdit::MoveMedia {
            media_id,
            folder_id,
        } => EditCommand::MoveMedia {
            media_id: media_id.clone(),
            folder_id: folder_id.clone(),
        },
    })
}

fn snapshots(project: &Project) -> Vec<ClipSnapshot> {
    project
        .tracks
        .iter()
        .flat_map(|track| {
            track.clips.iter().map(|clip| ClipSnapshot {
                track_id: track.id.clone(),
                clip: clip.clone(),
            })
        })
        .collect()
}

pub(crate) fn summarize(before: &Project, after: &Project, split: bool) -> EditStepReceipt {
    let old = snapshots(before);
    let new = snapshots(after);
    let old_ids: HashMap<_, _> = old
        .iter()
        .map(|item| (item.clip.id.as_str(), item))
        .collect();
    let new_ids: HashMap<_, _> = new
        .iter()
        .map(|item| (item.clip.id.as_str(), item))
        .collect();
    let mut receipt = EditStepReceipt {
        changed: !same_content(before, after),
        created_clips: new
            .iter()
            .filter(|item| !old_ids.contains_key(item.clip.id.as_str()))
            .cloned()
            .collect(),
        updated_clips: new
            .iter()
            .filter(|item| {
                old_ids
                    .get(item.clip.id.as_str())
                    .is_some_and(|old| *old != *item)
            })
            .cloned()
            .collect(),
        deleted_clips: old
            .iter()
            .filter(|item| !new_ids.contains_key(item.clip.id.as_str()))
            .cloned()
            .collect(),
        ..Default::default()
    };
    let old_tracks: HashSet<_> = before.tracks.iter().map(|track| &track.id).collect();
    let new_tracks: HashSet<_> = after.tracks.iter().map(|track| &track.id).collect();
    receipt.created_track_ids = after
        .tracks
        .iter()
        .filter(|track| !old_tracks.contains(&track.id))
        .map(|track| track.id.clone())
        .collect();
    receipt.deleted_track_ids = before
        .tracks
        .iter()
        .filter(|track| !new_tracks.contains(&track.id))
        .map(|track| track.id.clone())
        .collect();
    if split {
        for left in &receipt.updated_clips {
            let old = old_ids[left.clip.id.as_str()];
            if let Some(right) = receipt.created_clips.iter().find(|right| {
                right.track_id == left.track_id
                    && is_split_piece(&old.clip, &left.clip, &right.clip)
            }) {
                receipt.splits.push(SplitReceipt {
                    original_id: old.clip.id.clone(),
                    left_id: left.clip.id.clone(),
                    right_id: right.clip.id.clone(),
                });
            }
        }
    }
    receipt
}
