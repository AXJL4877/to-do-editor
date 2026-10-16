use super::*;
use crate::transform::{hit_test, TransformGesture};
use todo_application::{AppAction, AppUpdate, EditorCommand, EditorSession, Workspace};
use todo_core::{find_clip, resolve_layers, BaseSize, EditCommand, PreviewHit};

impl EditorController {
    pub fn preview_gesture_active(&self) -> bool {
        self.preview_drag.is_some() || self.preview_pan.is_some()
    }

    /// Identity-transform size of a picture layer. Media fit their display aspect:
    /// the decoded aperture when it matches the path, else source metadata. Text
    /// uses its measured block, so its box hugs the words rather than the frame.
    pub(super) fn element_base(
        &self,
        editor: &EditorSession,
        hit: &PreviewHit,
        canvas: (f64, f64),
    ) -> Option<BaseSize> {
        match hit.media() {
            Some(media) => BaseSize::fit(
                self.aspect_for_source(media.path.as_deref(), hit.source_size)?,
                canvas,
            ),
            None => editor.text_base(hit),
        }
    }

    fn aspect_for_source(&self, path: Option<&str>, dims: Option<(u32, u32)>) -> Option<f64> {
        if let Some(path) = path {
            if let Some((cached, aspect)) = &self.preview_source_aspect {
                if cached == path {
                    return Some(*aspect);
                }
            }
        }
        dims.filter(|(width, height)| *width > 0 && *height > 0)
            .map(|(width, height)| f64::from(width) / f64::from(height))
    }

    pub(super) fn preview_navigation(&mut self, intent: &Intent) -> bool {
        match *intent {
            Intent::PreviewZoom(_)
            | Intent::PreviewWheel { .. }
            | Intent::PreviewPan { .. }
            | Intent::PreviewFit
            | Intent::PreviewOutputOnly(_)
                if self.preview_drag.is_some() =>
            {
                true
            }
            Intent::PreviewZoom(zoom) => {
                self.preview_viewport.zoom_to(
                    zoom,
                    (0.5, 0.5),
                    self.preview_size,
                    self.output_frame,
                );
                true
            }
            Intent::PreviewWheel { delta, x, y } => {
                if delta.is_finite() && self.preview_pan.is_none() {
                    let zoom =
                        self.preview_viewport.zoom() * 1.2_f64.powf(delta.clamp(-16.0, 16.0));
                    self.preview_viewport.zoom_to(
                        zoom,
                        (x, y),
                        self.preview_size,
                        self.output_frame,
                    );
                }
                true
            }
            Intent::PreviewPan { kind, x, y } => {
                if x.is_finite() && y.is_finite() {
                    if kind == PreviewPointerKind::Down {
                        self.preview_pan = Some((x, y));
                    } else if let Some(previous) = self.preview_pan {
                        self.preview_viewport
                            .pan_by((x - previous.0, y - previous.1));
                        self.preview_pan = (kind != PreviewPointerKind::Up).then_some((x, y));
                    }
                } else if kind == PreviewPointerKind::Up {
                    self.preview_pan = None;
                }
                true
            }
            Intent::PreviewFit => {
                self.preview_viewport.fit();
                true
            }
            Intent::PreviewOutputOnly(value) => {
                self.preview_viewport.set_output_only(value);
                true
            }
            _ => false,
        }
    }

    pub(super) fn preview_pointer(
        &mut self,
        workspace: &mut Workspace,
        kind: PreviewPointerKind,
        x: f64,
        y: f64,
    ) -> AppUpdate {
        if (!x.is_finite() || !y.is_finite()) && kind != PreviewPointerKind::Up {
            return AppUpdate::default();
        }
        if kind == PreviewPointerKind::Down {
            if self.preview_pan.is_some() || self.preview_viewport.output_only() {
                return AppUpdate::default();
            }
            let Some(projection) = self
                .preview_viewport
                .projection(self.preview_size, self.output_frame)
            else {
                return AppUpdate::default();
            };
            let canvas_size = projection.canvas_size(self.preview_size);
            let (x, y) = projection.to_canvas((x, y));
            let Some(editor) = workspace.editor() else {
                return AppUpdate::default();
            };
            // Covering tracks first: earlier tracks (text, then video) sit on top.
            let layers = resolve_layers(editor.project(), editor.playhead());
            let mut picked = None;
            for hit in layers.iter().rev() {
                let Some(found) = find_clip(editor.project(), &hit.clip.id) else {
                    continue;
                };
                if found.track.locked {
                    continue;
                }
                let Some(base) = self.element_base(editor, hit, canvas_size) else {
                    continue;
                };
                let Some(bounds) = hit.clip.transform.bounds_for_base(base) else {
                    continue;
                };
                let selected = self.timeline.selected_ids().contains(&hit.clip.id);
                if let Some(zone) = hit_test(bounds, (x, y), canvas_size, selected) {
                    picked = Some((hit.clip.id.clone(), base, zone));
                    break;
                }
            }
            let Some((id, base, hit)) = picked else {
                return AppUpdate::default();
            };
            let peers = layers
                .iter()
                .filter(|layer| layer.clip.id != id)
                .filter_map(|layer| {
                    let base = self.element_base(editor, layer, canvas_size)?;
                    layer.clip.transform.bounds_for_base(base)
                })
                .collect();
            let playing = editor.playing();
            // A native no-activate popup can capture before a numeric field
            // reports lost_focus. Commit it before reading the gesture base.
            let _ = workspace.dispatch(AppAction::Editor(EditorCommand::CommitEdit));
            let editor = workspace.editor().expect("editor already checked");
            let Some(found) = find_clip(editor.project(), &id) else {
                return AppUpdate::default();
            };
            self.timeline.select_clip(editor.project(), &id, false);
            self.focus = EditorFocus::Timeline;
            self.preview_drag = Some(PreviewDrag {
                clip_id: id,
                gesture: TransformGesture::new(found.clip.transform, base, (x, y), hit),
                projection,
                canvas_size,
                peers,
                guides: Vec::new(),
            });
            self.timeline.cancel_drag();
            let mut update = workspace.dispatch(AppAction::Editor(EditorCommand::BeginEdit));
            if playing {
                update.effects.extend(
                    workspace
                        .dispatch(AppAction::Editor(EditorCommand::TogglePlay))
                        .effects,
                );
            }
            return update;
        }
        let Some(mut drag) = self.preview_drag.clone() else {
            return AppUpdate::default();
        };
        let update_result = (x.is_finite() && y.is_finite())
            .then(|| {
                drag.gesture.update(
                    drag.projection.to_canvas((x, y)),
                    drag.canvas_size,
                    &drag.peers,
                    self.timeline.snapping(),
                )
            })
            .flatten();
        if let Some(result) = &update_result {
            drag.guides = result.guides.clone();
        } else if kind != PreviewPointerKind::Up {
            drag.guides.clear();
        }
        let mut update = update_result
            .as_ref()
            .map(|result| {
                workspace.dispatch(AppAction::Editor(EditorCommand::Edit(
                    EditCommand::SetTransform {
                        clip_id: drag.clip_id.clone(),
                        transform: result.transform,
                    },
                )))
            })
            .unwrap_or_default();
        if kind == PreviewPointerKind::Up {
            // Capture release must finish even when its final coordinate cannot
            // produce a representable transform. Keep the last valid state.
            self.preview_drag = None;
            update.effects.extend(
                workspace
                    .dispatch(AppAction::Editor(EditorCommand::CommitEdit))
                    .effects,
            );
        } else {
            self.preview_drag = Some(drag);
        }
        update
    }
}
