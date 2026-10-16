use egui::{Response, Ui};
use todo_interaction::Intent;

/// Group the live updates from one numeric control into one application edit.
/// Typed values stay in the same transaction until focus leaves the control.
pub(crate) fn edit_response(
    ui: &Ui,
    response: &Response,
    change: Option<Intent>,
    intents: &mut Vec<Intent>,
) {
    let key = response.id.with("edit_transaction");
    let mut editing = ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
    if !editing && (response.drag_started() || change.is_some()) {
        intents.push(Intent::BeginEdit);
        editing = true;
    }
    if let Some(change) = change {
        intents.push(change);
    }
    if editing
        && (response.drag_stopped()
            || response.lost_focus()
            || (!response.dragged()
                && !response.has_focus()
                && !ui.input(|input| input.pointer.primary_down())))
    {
        intents.push(Intent::CommitEdit);
        editing = false;
    }
    ui.data_mut(|data| data.insert_temp(key, editing));
}
