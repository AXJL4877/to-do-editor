//! Motion edits: a clip's intro, outro and incoming transition, its audio
//! envelope, and the presets a project carries.
use crate::find_clip;
use crate::model::effects::find_motion;
use crate::query::adjacent_predecessor;
use crate::{ClipAudio, DomainError, MotionKind, MotionPreset, MotionRef, Project, TrackKind};
use todo_motion::{is_builtin_motion, MAX_MOTION_SECS};

/// Copy presets into the project (replacing same-id copies). Built-in ids are
/// never stored: they always mean the built-in.
pub(super) fn add_motions(
    mut project: Project,
    presets: Vec<MotionPreset>,
) -> Result<Project, DomainError> {
    for preset in presets {
        preset
            .validate()
            .map_err(|_| DomainError::InvalidMotion(preset.id.clone()))?;
        if is_builtin_motion(&preset.id) {
            continue;
        }
        project.motions.retain(|existing| existing.id != preset.id);
        project.motions.push(preset);
    }
    Ok(project)
}

fn check_ref(project: &Project, motion: &MotionRef, kind: MotionKind) -> Result<(), DomainError> {
    if find_motion(project, &motion.kind).is_none_or(|preset| preset.kind != kind) {
        return Err(DomainError::MotionNotFound(motion.kind.clone()));
    }
    if !motion.duration.is_finite() || motion.duration <= 0.0 || motion.duration > MAX_MOTION_SECS {
        return Err(DomainError::InvalidDuration);
    }
    Ok(())
}

/// Find a clip on an unlocked track and edit it in place.
fn edit_clip(
    mut project: Project,
    clip_id: &str,
    edit: impl FnOnce(&Project, TrackKind, &crate::Track) -> Result<(), DomainError>,
    apply: impl FnOnce(&mut crate::Clip),
) -> Result<Project, DomainError> {
    let (track_index, clip_index) = project
        .tracks
        .iter()
        .enumerate()
        .find_map(|(t, track)| {
            track
                .clips
                .iter()
                .position(|clip| clip.id == clip_id)
                .map(|c| (t, c))
        })
        .ok_or_else(|| DomainError::ClipNotFound(clip_id.into()))?;
    let track = &project.tracks[track_index];
    if track.locked {
        return Err(DomainError::TrackLocked(track.id.clone()));
    }
    edit(&project, track.kind, track)?;
    apply(&mut project.tracks[track_index].clips[clip_index]);
    Ok(project)
}

pub(super) fn set_intro(
    project: Project,
    clip_id: &str,
    intro: Option<MotionRef>,
) -> Result<Project, DomainError> {
    let check = intro.clone();
    edit_clip(
        project,
        clip_id,
        |project, kind, _| {
            if !kind.is_visual() {
                return Err(DomainError::UnsupportedClip(clip_id.into()));
            }
            check.map_or(Ok(()), |m| check_ref(project, &m, MotionKind::Intro))
        },
        |clip| clip.intro = intro,
    )
}

pub(super) fn set_outro(
    project: Project,
    clip_id: &str,
    outro: Option<MotionRef>,
) -> Result<Project, DomainError> {
    let check = outro.clone();
    edit_clip(
        project,
        clip_id,
        |project, kind, _| {
            if !kind.is_visual() {
                return Err(DomainError::UnsupportedClip(clip_id.into()));
            }
            check.map_or(Ok(()), |m| check_ref(project, &m, MotionKind::Outro))
        },
        |clip| clip.outro = outro,
    )
}

/// Set the transition into `clip_id`. It needs an adjacent predecessor on the
/// same picture track; removing one always succeeds.
pub(super) fn set_transition(
    project: Project,
    clip_id: &str,
    transition: Option<MotionRef>,
) -> Result<Project, DomainError> {
    let check = transition.clone();
    edit_clip(
        project,
        clip_id,
        |project, kind, track| {
            let Some(motion) = check else { return Ok(()) };
            if !kind.is_visual() {
                return Err(DomainError::UnsupportedClip(clip_id.into()));
            }
            check_ref(project, &motion, MotionKind::Transition)?;
            if adjacent_predecessor(track, clip_id).is_none() {
                return Err(DomainError::NoAdjacentClip(clip_id.into()));
            }
            Ok(())
        },
        |clip| clip.transition = transition,
    )
}

/// Show another library item in a media clip. Timing, motion and position
/// stay; a picture keeps its on-screen height (scale is converted between
/// the two aspect-fit sizes), so a figure standing on the ground stays there.
pub(super) fn replace_media(
    project: Project,
    clip_id: &str,
    media_id: &str,
) -> Result<Project, DomainError> {
    let canvas = (f64::from(project.width), f64::from(project.height));
    let media = project
        .media
        .iter()
        .find(|m| m.id == media_id)
        .cloned()
        .ok_or_else(|| DomainError::MediaNotFound(media_id.into()))?;
    let base_height = |item: &crate::MediaItem| -> Option<f64> {
        let (w, h) = (item.width?, item.height?);
        (w > 0 && h > 0)
            .then(|| crate::BaseSize::fit(f64::from(w) / f64::from(h), canvas))
            .flatten()
            .map(|base| base.height)
    };
    let old_height = find_clip(&project, clip_id)
        .and_then(|found| found.clip.media_id().map(str::to_owned))
        .and_then(|id| project.media.iter().find(|m| m.id == id))
        .and_then(base_height);
    let new_height = base_height(&media);
    edit_clip(
        project,
        clip_id,
        |_, kind, _| {
            let visual = crate::is_visual_media(&media);
            if kind.is_visual() != visual {
                return Err(DomainError::IncompatibleTrack(media.id.clone()));
            }
            Ok(())
        },
        |clip| {
            let duration = clip.duration;
            if let Some(source) = clip.media_mut() {
                source.media_id = media.id.clone();
                source.offset = if crate::is_image_media(&media) {
                    0.0
                } else {
                    source.offset.min((media.duration - duration).max(0.0))
                };
            }
            if let (Some(old), Some(new)) = (old_height, new_height) {
                if new > 0.0 {
                    clip.transform.scale *= old / new;
                }
            }
        },
    )
    .and_then(|project| {
        // Text clips have no media to replace.
        if find_clip(&project, clip_id).is_some_and(|f| f.clip.media_id() != Some(media_id)) {
            return Err(DomainError::UnsupportedClip(clip_id.into()));
        }
        Ok(project)
    })
}

pub(super) fn set_audio(
    project: Project,
    clip_id: &str,
    audio: Option<ClipAudio>,
) -> Result<Project, DomainError> {
    let valid = audio.is_none_or(|audio| {
        audio.gain.is_finite()
            && (0.0..=4.0).contains(&audio.gain)
            && audio.fade_in.is_finite()
            && audio.fade_in >= 0.0
            && audio.fade_out.is_finite()
            && audio.fade_out >= 0.0
    });
    edit_clip(
        project,
        clip_id,
        |_, kind, _| {
            if kind.is_visual() {
                return Err(DomainError::UnsupportedClip(clip_id.into()));
            }
            if !valid {
                return Err(DomainError::InvalidAudio);
            }
            Ok(())
        },
        |clip| clip.audio = audio,
    )
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn project() -> Project {
        let mut p = create_project(None, None, None, None, None);
        p.media.push(MediaItem {
            id: "img".into(),
            name: "a.png".into(),
            path: Some("a.png".into()),
            duration: 5.0,
            has_audio: Some(false),
            width: Some(1920),
            height: Some(1080),
            kind: Some(MediaKind::Image),
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        p = add_clip(
            p,
            "v1",
            normalize_clip("a".into(), "img".into(), 0.0, 2.0, 0.0, 1.0),
        );
        add_clip(
            p,
            "v1",
            normalize_clip("b".into(), "img".into(), 2.0, 2.0, 0.0, 1.0),
        )
    }

    fn set(p: Project, clip: &str, kind: &str, d: f64) -> Result<Project, DomainError> {
        execute_edit(
            p,
            EditCommand::SetTransition {
                clip_id: clip.into(),
                transition: Some(MotionRef::new(kind, d)),
            },
        )
    }

    #[test]
    fn dissolve_shows_both_clips_around_the_cut() {
        let p = set(project(), "b", "crossfade", 0.4).unwrap();
        let ids = |t: f64| {
            resolve_layers(&p, t)
                .into_iter()
                .map(|hit| (hit.clip.id.clone(), hit_transform(&p, &hit).opacity))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(1.5).len(), 1);
        let mid = ids(2.0);
        assert_eq!(
            mid.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!((mid[0].1 - 0.5).abs() < 1e-6 && (mid[1].1 - 0.5).abs() < 1e-6);
        // The window is centred on the cut: 0.2 s each side.
        assert_eq!(ids(1.81).len(), 2);
        assert_eq!(ids(2.19).len(), 2);
        assert_eq!(ids(2.21), vec![("b".to_string(), 1.0)]);
        // Export resolves the same stack.
        assert_eq!(export_layers(&p, 2.0).unwrap().len(), 2);
    }

    #[test]
    fn push_moves_both_clips_and_window_is_clamped() {
        let p = set(project(), "b", "pushLeft", 9.0).unwrap();
        let hits = resolve_layers(&p, 2.0);
        assert_eq!(hits[0].transition.as_ref().unwrap().duration, 2.0);
        let out = hit_transform(&p, &hits[0]);
        let into = hit_transform(&p, &hits[1]);
        assert!((out.x + 0.5).abs() < 1e-6, "{}", out.x);
        assert!((into.x - 0.5).abs() < 1e-6, "{}", into.x);
    }

    #[test]
    fn transitions_need_a_neighbour_and_a_transition_preset() {
        assert_eq!(
            set(project(), "a", "dissolve", 0.2).unwrap_err(),
            DomainError::NoAdjacentClip("a".into())
        );
        assert_eq!(
            set(project(), "b", "pop", 0.2).unwrap_err(),
            DomainError::MotionNotFound("pop".into())
        );
        // Moving the clip away keeps the transition but it stops playing.
        let p = set(project(), "b", "dissolve", 0.2).unwrap();
        let p = execute_edit(
            p,
            EditCommand::MoveClips {
                clip_ids: vec!["b".into()],
                delta: 1.0,
                track_id: None,
            },
        )
        .unwrap();
        assert!(find_clip(&p, "b").unwrap().clip.transition.is_some());
        assert_eq!(resolve_layers(&p, 2.95).len(), 0);
    }

    #[test]
    fn intro_clock_starts_when_the_clip_appears() {
        let p = set(project(), "b", "dissolve", 0.4).unwrap();
        let p = execute_edit(
            p,
            EditCommand::SetIntro {
                clip_id: "b".into(),
                intro: Some(MotionRef::new("rise", 0.4)),
            },
        )
        .unwrap();
        // 0.2 s before the cut the incoming clip is visible and its rise has begun.
        let hit = resolve_layers(&p, 1.8).pop().unwrap();
        assert_eq!(hit.clip.id, "b");
        let sample = hit_transform(&p, &hit);
        assert!(sample.y > 0.05);
        let settled = resolve_layers(&p, 2.21).pop().unwrap();
        assert!(hit_transform(&p, &settled).y.abs() < 1e-9);
    }

    #[test]
    fn library_presets_are_copied_in_and_builtins_are_not() {
        let mut custom = builtin_motions_for_test("crossfade");
        custom.id = "softWipe".into();
        let builtin = builtin_motions_for_test("dissolve");
        let p = execute_edit(
            project(),
            EditCommand::AddMotions {
                presets: vec![custom.clone(), builtin],
            },
        )
        .unwrap();
        assert_eq!(p.motions, vec![custom]);
        assert!(set(p, "b", "softWipe", 0.3).is_ok());
    }

    #[test]
    fn razor_keeps_entrance_left_and_exit_right() {
        let p = set(project(), "b", "dissolve", 0.2).unwrap();
        let p = execute_edit(
            p,
            EditCommand::SetOutro {
                clip_id: "b".into(),
                outro: Some(MotionRef::new("fadeOut", 0.3)),
            },
        )
        .unwrap();
        let p = razor(p, 3.0, Some("v1"), || "b2".into());
        let left = find_clip(&p, "b").unwrap().clip;
        let right = find_clip(&p, "b2").unwrap().clip;
        assert!(left.transition.is_some() && left.outro.is_none());
        assert!(right.transition.is_none() && right.outro.is_some());
    }

    #[test]
    fn replacing_a_picture_keeps_its_height_and_timing() {
        let mut p = project();
        p.media.push(MediaItem {
            id: "tall".into(),
            name: "tall.png".into(),
            path: Some("tall.png".into()),
            duration: 5.0,
            has_audio: Some(false),
            width: Some(500),
            height: Some(1000),
            kind: Some(MediaKind::Image),
            folder_id: None,
            alpha: None,
            reveal: None,
        });
        let p = execute_edit(
            p,
            EditCommand::SetTransform {
                clip_id: "b".into(),
                transform: Transform {
                    scale: 0.5,
                    ..Transform::default()
                },
            },
        )
        .unwrap();
        let before = find_clip(&p, "b").unwrap().clip;
        let p = execute_edit(
            p,
            EditCommand::ReplaceMedia {
                clip_id: "b".into(),
                media_id: "tall".into(),
            },
        )
        .unwrap();
        let after = find_clip(&p, "b").unwrap().clip;
        assert_eq!(after.media_id(), Some("tall"));
        assert_eq!(
            (after.start, after.duration),
            (before.start, before.duration)
        );
        // Both fit the full canvas height: the scale is unchanged.
        assert!((after.transform.scale - 0.5).abs() < 1e-9);
        assert_eq!(
            execute_edit(
                p,
                EditCommand::ReplaceMedia {
                    clip_id: "b".into(),
                    media_id: "nope".into(),
                },
            )
            .unwrap_err(),
            DomainError::MediaNotFound("nope".into())
        );
    }

    fn builtin_motions_for_test(id: &str) -> MotionPreset {
        todo_motion::builtin_motions()
            .iter()
            .find(|preset| preset.id == id)
            .unwrap()
            .clone()
    }
}
