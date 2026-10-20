use crate::EditorController;
use todo_application::{AppAction, AppUpdate, EditorCommand, Workspace};
use super::MotionSlot;
use todo_core::{find_clip, ClipTransform, EditCommand, MotionRef, TextContent, TrimEdge};
impl EditorController {
    pub(crate) fn property_duration(
        &self,
        workspace: &mut Workspace,
        clip_id: String,
        seconds: f64,
    ) -> AppUpdate {
        let time = workspace
            .editor()
            .and_then(|e| find_clip(e.edit_base(), &clip_id))
            .map(|f| f.clip.start + seconds)
            .unwrap_or(seconds);
        workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
            EditCommand::TrimClips {
                primary_id: clip_id,
                edge: TrimEdge::Out,
                time,
                linked: self.timeline.linked_selection(),
            },
        )))
    }
    pub(crate) fn property_text(
        &self,
        workspace: &mut Workspace,
        clip_id: String,
        content: TextContent,
    ) -> AppUpdate {
        workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
            EditCommand::SetText { clip_id, content },
        )))
    }
    /// Set or clear one motion of a clip; a library preset is copied into the
    /// project in the same step by the workspace.
    pub(crate) fn property_motion(
        &self,
        workspace: &mut Workspace,
        clip_id: String,
        slot: MotionSlot,
        motion: Option<MotionRef>,
    ) -> AppUpdate {
        let copy = motion
            .as_ref()
            .and_then(|motion| {
                let project = workspace.editor()?.project();
                let missing = todo_application::control::motions::missing_motions(
                    project,
                    workspace.motion_library(),
                    [motion],
                );
                (!missing.is_empty()).then_some(missing)
            });
        let edit = match slot {
            MotionSlot::Intro => EditCommand::SetIntro { clip_id, intro: motion },
            MotionSlot::Outro => EditCommand::SetOutro { clip_id, outro: motion },
            MotionSlot::Transition => EditCommand::SetTransition {
                clip_id,
                transition: motion,
            },
        };
        let commands = match copy {
            // One undo step: copy the preset in, then use it.
            Some(presets) => vec![EditCommand::AddMotions { presets }, edit],
            None => vec![edit],
        };
        workspace.dispatch(AppAction::Editor(EditorCommand::EditAll(commands)))
    }
    pub(crate) fn property_transform(
        &self,
        workspace: &mut Workspace,
        clip_id: String,
        transform: ClipTransform,
    ) -> AppUpdate {
        workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
            EditCommand::SetTransform { clip_id, transform },
        )))
    }
}
