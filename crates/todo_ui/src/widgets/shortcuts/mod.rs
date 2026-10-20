use egui::Context;
use todo_interaction::{EditorFocus, EditorViewModel, Intent, KeyCode, TimelineTool};

/// Collect editor shortcuts once, after text widgets have established focus.
pub(super) fn collect(ctx: &Context, vm: &EditorViewModel) -> Vec<Intent> {
    if ctx.wants_keyboard_input() {
        return Vec::new();
    }
    ctx.input(|input| {
        let mut intents = Vec::new();
        let modifiers = input.modifiers;
        let command = modifiers.ctrl || modifiers.command;
        if command && !modifiers.alt {
            for (key, intent) in [
                (egui::Key::A, Intent::SelectAll),
                (egui::Key::C, Intent::Copy),
                (egui::Key::V, Intent::Paste),
                (egui::Key::K, Intent::RazorAtPlayhead),
                (egui::Key::S, Intent::Save),
                (egui::Key::I, Intent::ImportPressed),
            ] {
                if input.key_pressed(key) {
                    intents.push(intent);
                }
            }
            if input.key_pressed(egui::Key::Z) {
                intents.push(if modifiers.shift {
                    Intent::Redo
                } else {
                    Intent::Undo
                });
            }
            if input.key_pressed(egui::Key::Y) {
                intents.push(Intent::Redo);
            }
        } else if !modifiers.alt {
            if input.key_pressed(egui::Key::V) {
                intents.push(Intent::ToolClicked(TimelineTool::Select));
            }
            if input.key_pressed(egui::Key::C) {
                intents.push(Intent::ToolClicked(TimelineTool::Razor));
            }
            if input.key_pressed(egui::Key::Space) {
                intents.push(Intent::TogglePlay);
            }
            // Ignore Delete while a pointer click is changing the active panel/row.
            // The view model still describes the selection before that click.
            let deleting = input.key_pressed(egui::Key::Delete)
                && !input.pointer.any_pressed()
                && !input.pointer.any_released();
            // Library: only Delete clears a row (Backspace is too easy after a click).
            // Timeline: Delete / Backspace still remove clips.
            if vm.focus == EditorFocus::Library {
                if deleting {
                    if let Some(media_id) = &vm.library.selected_id {
                        intents.push(Intent::RemoveMedia {
                            media_id: media_id.clone(),
                            cascade: false,
                        });
                    } else if let Some(id) = &vm.library.selected_folder_id {
                        intents.push(Intent::LibraryDeleteFolder { id: id.clone() });
                    }
                }
            } else if deleting || input.key_pressed(egui::Key::Backspace) {
                intents.push(Intent::DeleteSelection {
                    ripple: modifiers.shift,
                });
            }
        }
        if input.key_pressed(egui::Key::Escape) {
            intents.push(Intent::Key {
                code: KeyCode::Escape,
                typing: false,
            });
            intents.push(Intent::ClearSelection);
        }
        intents
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_core::Project;
    use todo_interaction::{EditorFocus, TimelineTool};

    fn view() -> EditorViewModel {
        crate::test_view(
            Project {
                version: 1,
                fps: 30.0,
                playhead: 0.0,
                width: 1920,
                height: 1080,
                duration: None,
                media_scale: Default::default(),
                media: Vec::new(),
                tracks: Vec::new(),
                text_styles: Vec::new(),
                motions: Vec::new(),
                folders: Vec::new(),
            },
            "test",
        )
    }

    fn press(
        vm: &EditorViewModel,
        key: egui::Key,
        modifiers: egui::Modifiers,
        typing: bool,
    ) -> Vec<Intent> {
        let ctx = Context::default();
        let mut output = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            if typing {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.text_edit_singleline(&mut String::new()).request_focus();
                });
            }
        });
        let _ = ctx.run(
            egui::RawInput {
                modifiers,
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| {
                if typing {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.text_edit_singleline(&mut String::new()).request_focus();
                    });
                }
                output = collect(ctx, vm);
            },
        );
        output
    }

    #[test]
    fn control_copy_and_paste_do_not_switch_tools() {
        let vm = view();
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let copy = press(&vm, egui::Key::C, ctrl, false);
        assert!(matches!(copy.as_slice(), [Intent::Copy]));
        let paste = press(&vm, egui::Key::V, ctrl, false);
        assert!(matches!(paste.as_slice(), [Intent::Paste]));
    }

    #[test]
    fn unmodified_tool_keys_still_select_tools() {
        assert!(matches!(
            press(&view(), egui::Key::V, egui::Modifiers::default(), false).as_slice(),
            [Intent::ToolClicked(TimelineTool::Select)]
        ));
        assert!(matches!(
            press(&view(), egui::Key::C, egui::Modifiers::default(), false).as_slice(),
            [Intent::ToolClicked(TimelineTool::Razor)]
        ));
    }

    #[test]
    fn delete_uses_panel_focus_and_shift_requests_ripple() {
        let mut vm = view();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        assert!(matches!(
            press(&vm, egui::Key::Delete, shift, false).as_slice(),
            [Intent::DeleteSelection { ripple: true }]
        ));
        vm.focus = EditorFocus::Library;
        vm.library.selected_id = Some("source-1".into());
        vm.library.selected_folder_id = Some("parent-bin".into());
        assert!(
            matches!(press(&vm, egui::Key::Delete, egui::Modifiers::default(), false).as_slice(),
            [Intent::RemoveMedia { media_id, cascade: false }] if media_id == "source-1")
        );
        assert!(
            press(&vm, egui::Key::Backspace, egui::Modifiers::default(), false).is_empty(),
            "Backspace must not clear library media after a click"
        );
    }

    #[test]
    fn text_edit_focus_suppresses_editor_shortcuts() {
        let vm = view();
        for key in [egui::Key::V, egui::Key::Space, egui::Key::Delete] {
            assert!(press(&vm, key, egui::Modifiers::default(), true).is_empty());
        }
    }

    #[test]
    fn library_delete_targets_selected_folder_without_deleting_timeline_selection() {
        let mut vm = view();
        vm.focus = EditorFocus::Library;
        vm.library.selected_folder_id = Some("bin".into());
        assert!(matches!(
            press(&vm, egui::Key::Delete, egui::Modifiers::default(), false).as_slice(),
            [Intent::LibraryDeleteFolder { id }] if id == "bin"
        ));
        assert!(press(&vm, egui::Key::Delete, egui::Modifiers::default(), true).is_empty());
        vm.library.selected_folder_id = None;
        assert!(press(&vm, egui::Key::Delete, egui::Modifiers::default(), false).is_empty());
    }

    #[test]
    fn pointer_selection_with_delete_cannot_delete_the_previous_library_row() {
        for pressed in [true, false] {
            let mut vm = view();
            vm.focus = EditorFocus::Library;
            vm.library.selected_id = Some("previous-row".into());
            let ctx = Context::default();
            let _ = ctx.run(egui::RawInput::default(), |_| {});
            if !pressed {
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::PointerButton {
                            pos: egui::pos2(40.0, 80.0),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        }],
                        ..Default::default()
                    },
                    |_| {},
                );
            }
            let mut intents = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerButton {
                            pos: egui::pos2(40.0, 80.0),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                        egui::Event::Key {
                            key: egui::Key::Delete,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| intents = collect(ctx, &vm),
            );
            assert!(
                intents.is_empty(),
                "selection changes on pointer pressed={pressed}: {intents:?}"
            );
        }
    }
}
