use super::*;
use crate::EditorCommand;
use todo_core::{execute_edit, EditCommand};

impl EditorSession {
    #[must_use]
    pub fn apply(&mut self, command: EditorCommand) -> EditorUpdate {
        let before_revision = self.revision;
        let mut update = EditorUpdate::default();
        let commands = match command {
            EditorCommand::BeginEdit => {
                if self.gesture_before.is_none() {
                    self.project.playhead = self.playhead;
                    self.gesture_before = Some(self.project.clone());
                }
                Vec::new()
            }
            EditorCommand::CommitEdit => {
                self.finish_edit();
                Vec::new()
            }
            EditorCommand::CancelEdit => {
                if let Some(before) = self.gesture_before.take() {
                    self.restore(before)
                } else {
                    Vec::new()
                }
            }
            EditorCommand::Edit(edit) => match execute_edit(self.edit_base_for(&edit), edit) {
                Ok(project) => return self.commit_project(project),
                Err(error) => {
                    update.error = Some(error.clone());
                    self.status = Some(Status::Domain(error));
                    Vec::new()
                }
            },
            EditorCommand::EditAll(edits) => {
                let mut project = self.project.clone();
                project.playhead = self.playhead;
                for edit in edits {
                    match execute_edit(project, edit) {
                        Ok(next) => project = next,
                        Err(error) => {
                            update.error = Some(error.clone());
                            self.status = Some(Status::Domain(error));
                            return update;
                        }
                    }
                }
                return self.commit_project(project);
            }
            EditorCommand::Undo => {
                let previous = self
                    .gesture_before
                    .take()
                    .or_else(|| self.history.undo(&self.project));
                previous.map(|p| self.restore(p)).unwrap_or_default()
            }
            EditorCommand::Redo => {
                self.finish_edit();
                self.history
                    .redo(&self.project)
                    .map(|p| self.restore(p))
                    .unwrap_or_default()
            }
            EditorCommand::Copy { clip_ids } => {
                self.clipboard.capture(&self.project, &clip_ids);
                self.status = Some(Status::Copied);
                Vec::new()
            }
            EditorCommand::Paste => {
                return self.apply(EditorCommand::Edit(EditCommand::Paste {
                    clips: self.clipboard.clips(),
                    time: self.playhead,
                }));
            }
            EditorCommand::RemoveMedia { media_id, cascade } => {
                match todo_core::remove_media(self.project.clone(), &media_id, cascade) {
                    Ok(project) => {
                        if self
                            .source_monitor
                            .as_ref()
                            .is_some_and(|s| s.media_id == media_id)
                        {
                            self.source_monitor = None;
                            self.monitor_pane = MonitorPane::Program;
                        }
                        self.note_edit(project)
                    }
                    Err(error) => {
                        update.error = Some(error.clone());
                        self.status = Some(Status::Domain(error));
                        Vec::new()
                    }
                }
            }
            EditorCommand::Seek(time) => self.scrub(time),
            EditorCommand::OpenSourceMonitor { media_id } => self.open_source_monitor(media_id),
            EditorCommand::ShowProgramMonitor => self.show_program_monitor(),
            EditorCommand::ShowSourceMonitor => self.show_source_monitor(),
            EditorCommand::TogglePlay => self.toggle_play(),
            EditorCommand::Frame { now } => self.frame(now),
            EditorCommand::Clock { epoch, now, sample } => self.on_clock(epoch, now, sample),
            EditorCommand::ClockFailed { epoch } => self.on_clock_failed(epoch),
            EditorCommand::RefreshPicture => {
                self.last_seek_ms = None;
                self.last_layers = None;
                self.emit(Want {
                    audio: false,
                    transport: false,
                    picture: true,
                })
            }
            EditorCommand::ImportStarted => {
                self.finish_edit();
                self.importing = true;
                self.playing = false;
                self.emit(Want {
                    audio: false,
                    transport: true,
                    picture: false,
                })
            }
            EditorCommand::Imported(media) => {
                return self.import_media(media, true);
            }
            EditorCommand::RegisterMedia(media) => {
                return self.import_media(media, false);
            }
            EditorCommand::AdoptMedia(media) => {
                return self.adopt_media(media);
            }
            EditorCommand::ImportFinished => {
                self.importing = false;
                Vec::new()
            }
            EditorCommand::Saved(project) => {
                self.saved_project = project;
                Vec::new()
            }
        };
        update.commands = commands;
        update.content_changed = self.revision != before_revision;
        update
    }

    fn edit_base_for(&self, edit: &EditCommand) -> Project {
        let mut base = self.edit_base().clone();
        if let EditCommand::MoveClips {
            track_id: Some(id), ..
        } = edit
        {
            if self.gesture_before.is_some() && !base.tracks.iter().any(|track| track.id == *id) {
                if let Some(index) = self.project.tracks.iter().position(|track| track.id == *id) {
                    // Keep an explicitly hovered generated lane available in
                    // this execution copy, without changing the undo baseline.
                    let mut target = self.project.tracks[index].clone();
                    target.clips.clear();
                    let insert = self.project.tracks[index + 1..]
                        .iter()
                        .find_map(|next| base.tracks.iter().position(|track| track.id == next.id))
                        .unwrap_or(base.tracks.len());
                    base.tracks.insert(insert, target);
                }
            }
        }
        base
    }

    pub(crate) fn finish_edit(&mut self) {
        if let Some(before) = self.gesture_before.take() {
            self.history.record(before, &self.project);
        }
    }
    pub(crate) fn commit_project(&mut self, project: Project) -> EditorUpdate {
        let old_ids: std::collections::HashSet<_> = self
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.id.as_str())
            .collect();
        let new_clips = project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| !old_ids.contains(clip.id.as_str()))
            .map(|clip| clip.id.clone())
            .collect();
        let content_changed = !same_content(&self.project, &project);
        self.status = None;
        let commands = self.note_edit(project);
        EditorUpdate {
            commands,
            new_clips,
            content_changed,
            error: None,
        }
    }
    fn note_edit(&mut self, project: Project) -> Vec<PreviewCommand> {
        if !same_content(&self.project, &project) {
            self.revision = self.revision.wrapping_add(1);
        }
        if self.gesture_before.is_none() {
            self.history.record(self.project.clone(), &project);
        }
        let moved = playhead_changed(project.playhead, self.synced_playhead);
        self.project = project;
        if moved {
            self.playhead = self.project.playhead;
            self.synced_playhead = self.playhead;
        }
        self.emit(Want {
            audio: true,
            transport: moved,
            picture: true,
        })
    }
    fn restore(&mut self, project: Project) -> Vec<PreviewCommand> {
        if !same_content(&self.project, &project) {
            self.revision = self.revision.wrapping_add(1);
        }
        self.project = project;
        self.playing = false;
        self.playhead = self.project.playhead;
        self.synced_playhead = self.playhead;
        self.last_tick = None;
        self.sync()
    }

    fn open_source_monitor(&mut self, media_id: String) -> Vec<PreviewCommand> {
        let Some(media) = self.project.media.iter().find(|m| m.id == media_id) else {
            return Vec::new();
        };
        if !todo_core::is_visual_media(media) {
            return Vec::new();
        }
        let path_ok = media.path.as_ref().is_some_and(|p| !p.trim().is_empty());
        if !path_ok {
            return Vec::new();
        }
        self.playing = false;
        self.last_tick = None;
        self.source_monitor = Some(SourceMonitor {
            media_id,
            time: 0.0,
        });
        self.monitor_pane = MonitorPane::Source;
        self.last_layers = None;
        self.emit(Want {
            audio: true,
            transport: true,
            picture: true,
        })
    }

    fn show_program_monitor(&mut self) -> Vec<PreviewCommand> {
        if self.monitor_pane == MonitorPane::Program {
            return Vec::new();
        }
        self.playing = false;
        self.last_tick = None;
        self.monitor_pane = MonitorPane::Program;
        self.last_layers = None;
        self.emit(Want {
            audio: true,
            transport: true,
            picture: true,
        })
    }

    fn show_source_monitor(&mut self) -> Vec<PreviewCommand> {
        if self.source_monitor.is_none() {
            return Vec::new();
        }
        if self.monitor_pane == MonitorPane::Source {
            return Vec::new();
        }
        self.playing = false;
        self.last_tick = None;
        self.monitor_pane = MonitorPane::Source;
        self.last_layers = None;
        self.emit(Want {
            audio: true,
            transport: true,
            picture: true,
        })
    }
}
