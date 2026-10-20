use super::library::{create_folder, delete_folder, move_folder, move_media, rename_folder};
use super::{
    add_text, paste_clips, set_text, stretch_clips, transform::set_transform, trim_clips,
    CopiedClip,
};
use crate::{
    add_track, clip_end, compact_tracks, delete_clips, delete_track, find_clip, insert_track_below,
    linked_clip_ids, move_track, place_media, place_media_range, razor_clips, set_media_scale_mode,
    set_project_duration, set_project_fps, set_project_output_size, validate_project, ClipAudio,
    DomainError, MediaScaleMode, MotionPreset, MotionRef, Project, TextContent, TrackKind,
    Transform, TrimEdge,
};

/// Semantic project edits; no input-device or layout concepts cross this boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum EditCommand {
    PlaceMedia {
        media_id: String,
        track_id: Option<String>,
        time: f64,
    },
    PlaceMediaRange {
        media_id: String,
        track_id: Option<String>,
        time: f64,
        source_in: f64,
        source_out: f64,
    },
    MoveClips {
        clip_ids: Vec<String>,
        delta: f64,
        track_id: Option<String>,
    },
    TrimClips {
        primary_id: String,
        edge: TrimEdge,
        time: f64,
        linked: bool,
    },
    StretchClips {
        primary_id: String,
        edge: TrimEdge,
        time: f64,
        linked: bool,
    },
    Split {
        clip_ids: Vec<String>,
        time: f64,
    },
    /// Split selected clips and their linked partners. `Split` keeps explicit selection semantics.
    SplitLinked {
        clip_ids: Vec<String>,
        time: f64,
    },
    DeleteClips {
        clip_ids: Vec<String>,
        ripple: bool,
    },
    AddTrack {
        kind: TrackKind,
    },
    InsertTrackBelow {
        track_id: String,
    },
    /// Swap with the adjacent same-kind neighbor (`up` = toward the top of the list).
    MoveTrack {
        track_id: String,
        up: bool,
    },
    /// Pack adjacent unlocked tracks while preserving overlap order.
    CompactTracks,
    DeleteTrack {
        track_id: String,
    },
    ToggleMute {
        track_id: String,
    },
    ToggleLock {
        track_id: String,
    },
    SetDuration {
        seconds: f64,
    },
    SetOutputSize {
        width: u32,
        height: u32,
    },
    SetFps {
        fps: f64,
    },
    SetMediaScaleMode {
        mode: MediaScaleMode,
    },
    SetTransform {
        clip_id: String,
        transform: Transform,
    },
    Paste {
        clips: Vec<CopiedClip>,
        time: f64,
    },
    AddText {
        track_id: Option<String>,
        time: f64,
        duration: f64,
        content: TextContent,
    },
    SetText {
        clip_id: String,
        content: TextContent,
    },
    CreateFolder {
        id: String,
        name: String,
        parent_id: Option<String>,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    DeleteFolder {
        id: String,
    },
    MoveFolder {
        id: String,
        parent_id: Option<String>,
    },
    MoveMedia {
        media_id: String,
        folder_id: Option<String>,
    },
    /// One progressive-placement batch of an AI production.
    ApplyGenerated(Vec<super::GeneratedEdit>),
    /// Copy library presets into the project before clips use them.
    AddMotions {
        presets: Vec<MotionPreset>,
    },
    SetIntro {
        clip_id: String,
        intro: Option<MotionRef>,
    },
    SetOutro {
        clip_id: String,
        outro: Option<MotionRef>,
    },
    /// Transition into the clip from its adjacent same-track predecessor.
    SetTransition {
        clip_id: String,
        transition: Option<MotionRef>,
    },
    SetAudio {
        clip_id: String,
        audio: Option<ClipAudio>,
    },
    /// Show another library item in a media clip (timing and motion stay).
    ReplaceMedia {
        clip_id: String,
        media_id: String,
    },
}

fn require_clips(project: &Project, ids: &[String]) -> Result<(), DomainError> {
    if ids.is_empty() {
        return Err(DomainError::InvalidProject("clip_ids"));
    }
    for id in ids {
        let found = find_clip(project, id).ok_or_else(|| DomainError::ClipNotFound(id.clone()))?;
        if found.track.locked {
            return Err(DomainError::TrackLocked(found.track_id));
        }
    }
    Ok(())
}

pub fn execute_edit(project: Project, command: EditCommand) -> Result<Project, DomainError> {
    validate_project(&project)?;
    let next = match command {
        EditCommand::PlaceMedia {
            media_id,
            track_id,
            time,
        } => place_media(project, &media_id, track_id.as_deref(), time)?,
        EditCommand::PlaceMediaRange {
            media_id,
            track_id,
            time,
            source_in,
            source_out,
        } => place_media_range(
            project,
            &media_id,
            track_id.as_deref(),
            time,
            source_in,
            source_out,
        )?,
        EditCommand::MoveClips {
            clip_ids,
            delta,
            track_id,
        } => super::move_clips::try_move_clips(project, &clip_ids, delta, track_id.as_deref())?,
        EditCommand::TrimClips {
            primary_id,
            edge,
            time,
            linked,
        } => trim_clips(project, &primary_id, edge, time, linked)?,
        EditCommand::StretchClips {
            primary_id,
            edge,
            time,
            linked,
        } => stretch_clips(project, &primary_id, edge, time, linked)?,
        EditCommand::SplitLinked { clip_ids, time } => {
            require_clips(&project, &clip_ids)?;
            let clip_ids = linked_clip_ids(&project, &clip_ids);
            execute_edit(project, EditCommand::Split { clip_ids, time })?
        }
        EditCommand::Split { clip_ids, time } => {
            if !time.is_finite() || time < 0.0 {
                return Err(DomainError::InvalidTime);
            }
            require_clips(&project, &clip_ids)?;
            require_clips(&project, &linked_clip_ids(&project, &clip_ids))?;
            for id in &clip_ids {
                let clip = find_clip(&project, id).unwrap().clip;
                let minimum = super::minimum_duration(&project);
                if time > clip.start
                    && time < clip_end(&clip)
                    && (time - clip.start + 1e-9 < minimum
                        || clip_end(&clip) - time + 1e-9 < minimum)
                {
                    return Err(DomainError::ClipTooShort(id.clone()));
                }
            }
            razor_clips(project, time, &clip_ids)
        }
        EditCommand::DeleteClips { clip_ids, ripple } => {
            require_clips(&project, &clip_ids)?;
            let next = delete_clips(project.clone(), &clip_ids, ripple);
            if ripple && next == project {
                return Err(DomainError::RippleConflict);
            }
            next
        }
        EditCommand::AddTrack { kind } => add_track(project, kind),
        EditCommand::InsertTrackBelow { track_id } => insert_track_below(project, &track_id)?,
        EditCommand::MoveTrack { track_id, up } => move_track(project, &track_id, up)?,
        EditCommand::CompactTracks => compact_tracks(project),
        EditCommand::DeleteTrack { track_id } => {
            let track = project
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .ok_or_else(|| DomainError::TrackNotFound(track_id.clone()))?;
            if track.locked {
                return Err(DomainError::TrackLocked(track_id));
            }
            if project.tracks.len() <= 1
                || (matches!(track.kind, TrackKind::Video | TrackKind::Audio)
                    && project
                        .tracks
                        .iter()
                        .filter(|t| t.kind == track.kind)
                        .count()
                        <= 1)
            {
                return Err(DomainError::TrackRequired(track.kind));
            }
            delete_track(project, &track_id)
        }
        EditCommand::ToggleMute { track_id } => {
            let mut next = project;
            let track = next
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
                .ok_or_else(|| DomainError::TrackNotFound(track_id.clone()))?;
            track.muted = !track.muted;
            next
        }
        EditCommand::ToggleLock { track_id } => {
            let mut next = project;
            let track = next
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
                .ok_or_else(|| DomainError::TrackNotFound(track_id.clone()))?;
            track.locked = !track.locked;
            next
        }
        EditCommand::SetDuration { seconds } => set_project_duration(project, seconds)?,
        EditCommand::SetOutputSize { width, height } => {
            set_project_output_size(project, width, height)?
        }
        EditCommand::SetFps { fps } => set_project_fps(project, fps)?,
        EditCommand::SetMediaScaleMode { mode } => set_media_scale_mode(project, mode),
        EditCommand::SetTransform { clip_id, transform } => {
            set_transform(project, &clip_id, transform)?
        }
        EditCommand::Paste { clips, time } => paste_clips(project, &clips, time)?,
        EditCommand::AddText {
            track_id,
            time,
            duration,
            content,
        } => add_text(project, track_id.as_deref(), time, duration, content)?,
        EditCommand::SetText { clip_id, content } => set_text(project, &clip_id, content)?,
        EditCommand::CreateFolder {
            id,
            name,
            parent_id,
        } => create_folder(project, id, name, parent_id)?,
        EditCommand::RenameFolder { id, name } => rename_folder(project, &id, name)?,
        EditCommand::DeleteFolder { id } => delete_folder(project, &id)?,
        EditCommand::MoveFolder { id, parent_id } => move_folder(project, &id, parent_id)?,
        EditCommand::MoveMedia {
            media_id,
            folder_id,
        } => move_media(project, &media_id, folder_id)?,
        EditCommand::ApplyGenerated(edits) => super::apply_generated(project, edits)?,
        EditCommand::AddMotions { presets } => super::motion::add_motions(project, presets)?,
        EditCommand::SetIntro { clip_id, intro } => {
            super::motion::set_intro(project, &clip_id, intro)?
        }
        EditCommand::SetOutro { clip_id, outro } => {
            super::motion::set_outro(project, &clip_id, outro)?
        }
        EditCommand::SetTransition {
            clip_id,
            transition,
        } => super::motion::set_transition(project, &clip_id, transition)?,
        EditCommand::SetAudio { clip_id, audio } => {
            super::motion::set_audio(project, &clip_id, audio)?
        }
        EditCommand::ReplaceMedia { clip_id, media_id } => {
            super::motion::replace_media(project, &clip_id, &media_id)?
        }
    };
    validate_project(&next)?;
    Ok(next)
}
