use crate::EditorController;
use todo_application::{AppAction, AppUpdate, EditorCommand, Workspace};
use todo_core::{find_clip, ClipTransform, EditCommand, TextContent, TrimEdge};
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
