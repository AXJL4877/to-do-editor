//! The motion presets an editing session may use beyond the built-ins (the
//! user's and extensions' libraries, loaded by the app), and the rule that
//! copies a preset into a project the first time a clip names it.
use crate::{EditCommand, Project};
use todo_core::{find_motion, motion::is_builtin_motion, MotionPreset, MotionRef};

/// Presets in `catalog` that `refs` name and `project` does not carry yet.
pub fn missing_motions<'a>(
    project: &Project,
    catalog: &[MotionPreset],
    refs: impl IntoIterator<Item = &'a MotionRef>,
) -> Vec<MotionPreset> {
    let mut out: Vec<MotionPreset> = Vec::new();
    for motion in refs {
        if is_builtin_motion(&motion.kind)
            || find_motion(project, &motion.kind).is_some()
            || out.iter().any(|preset| preset.id == motion.kind)
        {
            continue;
        }
        if let Some(preset) = catalog.iter().find(|preset| preset.id == motion.kind) {
            out.push(preset.clone());
        }
    }
    out
}

/// The copy-in step to run before an edit naming `refs`, if one is needed.
pub(crate) fn copy_in<'a>(
    project: &Project,
    catalog: &[MotionPreset],
    refs: impl IntoIterator<Item = &'a MotionRef>,
) -> Option<EditCommand> {
    let presets = missing_motions(project, catalog, refs);
    (!presets.is_empty()).then_some(EditCommand::AddMotions { presets })
}
