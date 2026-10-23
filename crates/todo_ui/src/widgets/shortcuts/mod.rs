use egui::Context;
use todo_interaction::{EditorFocus, EditorViewModel, Intent, KeyCode, TimelineTool};

fn key_event(input: &egui::InputState, key: egui::Key) -> Option<egui::Modifiers> {
    input.events.iter().find_map(|event| match event {
        egui::Event::Key {
            key: k,
            pressed: true,
            modifiers,
            ..
        } if *k == key => Some(*modifiers),
        _ => None,
    })
}

/// Collect editor shortcuts once, after text widgets have established focus.
pub(super) fn collect(ctx: &Context, vm: &EditorViewModel) -> Vec<Intent> {
    let typing = ctx.wants_keyboard_input();
    let library_has_media = !vm.library.selected_ids.is_empty()
        || vm.library.selected_id.is_some();
    let library_panel = vm.focus == EditorFocus::Library;
    // Media tab handles these before its search TextEdit; skip duplicates.
    let library_done = super::library::library_keys_already_handled(ctx);

    ctx.input(|input| {
        let mut intents = Vec::new();
        let modifiers = input.modifiers;
        let command = modifiers.ctrl || modifiers.command;
        let pointer_busy = input.pointer.any_pressed() || input.pointer.any_released();
        let delete_mods = key_event(input, egui::Key::Delete)
            .or_else(|| input.key_pressed(egui::Key::Delete).then_some(modifiers));
        let deleting = delete_mods.is_some() && !pointer_busy;
        let select_all = key_event(input, egui::Key::A)
            .or_else(|| input.key_pressed(egui::Key::A).then_some(modifiers))
            .is_some_and(|mods| (mods.ctrl || mods.command) && !mods.alt);

        if !library_done {
            if library_has_media && deleting {
                let media_ids: Vec<String> = if !vm.library.selected_ids.is_empty() {
                    vm.library.selected_ids.clone()
                } else if let Some(id) = &vm.library.selected_id {
                    vec![id.clone()]
                } else {
                    Vec::new()
                };
                for media_id in media_ids {
                    intents.push(Intent::RemoveMedia {
                        media_id,
                        cascade: true,
                    });
                }
            }

            if library_panel && select_all {
                intents.push(Intent::SelectAll);
            }
        }

        if typing {
            return intents;
        }

        if !library_done && library_panel && deleting && !library_has_media {
            if let Some(id) = &vm.library.selected_folder_id {
                intents.push(Intent::LibraryDeleteFolder { id: id.clone() });
            }
        }

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
                    if matches!(intent, Intent::SelectAll)
                        && (library_panel || library_done)
                    {
                        continue;
                    }
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
            if !(library_panel || library_has_media)
                && (deleting || input.key_pressed(egui::Key::Backspace))
            {
                intents.push(Intent::DeleteSelection {
                    ripple: delete_mods.unwrap_or(modifiers).shift,
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
                tracks: Vec::new(),
                text_styles: Vec::new(),
                motions: Vec::new(),
                media: Vec::new(),
                folders: Vec::new(),
            },
            "test",
        )
    }

    fn press(
        vm: &EditorViewModel,
        key: egui::Key,
        modifiers: egui::Modifiers,
        wants_keyboard: bool,
    ) -> Vec<Intent> {
        let ctx = Context::default();
        let mut output = Vec::new();
        if wants_keyboard {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut text = String::new();
                    ui.text_edit_singleline(&mut text).request_focus();
                });
            });
        }
        let _ = ctx.run(
            egui::RawInput {
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
                if wants_keyboard {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut text = String::new();
                        ui.text_edit_singleline(&mut text);
                    });
                }
                output = collect(ctx, vm);
            },
        );
        output
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
            [Intent::RemoveMedia { media_id, cascade: true }] if media_id == "source-1")
        );
        vm.library.selected_ids = vec!["a".into(), "b".into()];
        assert!(matches!(
            press(&vm, egui::Key::Delete, egui::Modifiers::default(), false).as_slice(),
            [
                Intent::RemoveMedia {
                    media_id: a,
                    cascade: true
                },
                Intent::RemoveMedia {
                    media_id: b,
                    cascade: true
                }
            ] if a == "a" && b == "b"
        ));
        assert!(
            press(&vm, egui::Key::Backspace, egui::Modifiers::default(), false).is_empty(),
            "Backspace must not clear library media after a click"
        );
    }

    #[test]
    fn library_selection_delete_works_while_search_still_wants_keyboard() {
        let mut vm = view();
        vm.focus = EditorFocus::Timeline;
        vm.library.selected_ids = vec!["clip-a".into(), "clip-b".into()];
        let intents = press(&vm, egui::Key::Delete, egui::Modifiers::default(), true);
        assert!(
            matches!(
                intents.as_slice(),
                [
                    Intent::RemoveMedia {
                        media_id: a,
                        cascade: true
                    },
                    Intent::RemoveMedia {
                        media_id: b,
                        cascade: true
                    }
                ] if a == "clip-a" && b == "clip-b"
            ),
            "{intents:?}"
        );
    }

    #[test]
    fn library_ctrl_a_works_while_search_still_wants_keyboard() {
        let mut vm = view();
        vm.focus = EditorFocus::Library;
        let command = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let intents = press(&vm, egui::Key::A, command, true);
        assert!(
            matches!(intents.as_slice(), [Intent::SelectAll]),
            "{intents:?}"
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
