use crate::editor::*;
use todo_core::project_duration;

impl EditorSession {
    pub(crate) fn toggle_play(&mut self) -> Vec<PreviewCommand> {
        if (self.importing || !self.can_play_at(self.picture().active)) && !self.playing {
            return Vec::new();
        }
        let restart_source = !self.playing && self.showing_source() && {
            let media_id = self
                .source_monitor
                .as_ref()
                .map(|source| source.media_id.as_str())
                .unwrap_or_default();
            let end = self
                .project
                .media
                .iter()
                .find(|media| media.id == media_id)
                .map(|media| media.duration.max(0.0))
                .unwrap_or(0.0);
            let at_end = self
                .source_monitor
                .as_ref()
                .is_some_and(|source| source.time >= (end - 1e-4).max(0.0));
            if at_end {
                if let Some(source) = self.source_monitor.as_mut() {
                    source.time = 0.0;
                }
                true
            } else {
                false
            }
        };
        self.playing = !self.playing;
        self.last_tick = None;
        self.audio_unavailable = false;
        self.clock_missing_at = None;
        self.emit(Want {
            audio: false,
            transport: true,
            picture: restart_source,
        })
    }

    pub(crate) fn scrub(&mut self, time: f64) -> Vec<PreviewCommand> {
        // Timeline seeks always drive the program monitor.
        let left_source = self.showing_source();
        self.monitor_pane = MonitorPane::Program;
        let was_playing = self.playing;
        self.playing = false;
        self.last_tick = None;
        // Seeking changes no project content; keep media and timeline storage in
        // place on the pointer-event hot path. Match the domain seek's clamp and
        // epsilon behavior, including NaN mapping to the start of the timeline.
        let clamped = time.max(0.0).min(project_duration(&self.project));
        let next = if (clamped - self.project.playhead).abs() < f64::EPSILON {
            self.project.playhead
        } else {
            clamped
        };
        let moved = playhead_changed(next, self.playhead);
        self.project.playhead = next;
        self.playhead = self.project.playhead;
        self.synced_playhead = self.playhead;
        self.emit(Want {
            audio: left_source,
            transport: was_playing || moved || left_source,
            picture: true,
        })
    }

    pub(crate) fn frame(&mut self, now: f64) -> Vec<PreviewCommand> {
        if !self.playing || !now.is_finite() {
            return Vec::new();
        }
        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return Vec::new();
        };
        let dt = (now - last).max(0.0);
        self.last_tick = Some(now);
        if self.showing_source() {
            return self.frame_source(now, dt);
        }
        let next = self.integrated_time(now, dt);
        let end = project_duration(&self.project);
        if next >= end {
            self.playing = false;
            self.last_tick = None;
            return self.scrub(end);
        }
        if !playhead_changed(next, self.playhead) {
            return Vec::new();
        }
        self.playhead = next;
        self.emit(Want {
            audio: false,
            transport: false,
            picture: true,
        })
    }

    fn frame_source(&mut self, _now: f64, dt: f64) -> Vec<PreviewCommand> {
        let media_id = self
            .source_monitor
            .as_ref()
            .map(|s| s.media_id.clone())
            .unwrap_or_default();
        let end = self
            .project
            .media
            .iter()
            .find(|m| m.id == media_id)
            .map(|m| m.duration.max(0.0))
            .unwrap_or(0.0);
        let next = self.integrated_source_time(dt);
        let previous = self.source_monitor.as_ref().map(|s| s.time).unwrap_or(0.0);
        if next >= end {
            if let Some(source) = self.source_monitor.as_mut() {
                source.time = end;
            }
            self.playing = false;
            self.last_tick = None;
            return self.emit(Want {
                audio: false,
                transport: true,
                picture: true,
            });
        }
        if let Some(source) = self.source_monitor.as_mut() {
            source.time = next;
        }
        if !playhead_changed(next, previous) {
            return Vec::new();
        }
        self.emit(Want {
            audio: false,
            transport: false,
            picture: true,
        })
    }

    pub(crate) fn integrated_time(&mut self, now: f64, dt: f64) -> f64 {
        if let Some(clock) = self.clock {
            let age = now - clock.received_at;
            if (0.0..CLOCK_FRESH_SECS).contains(&age) {
                let extra = if clock.playing { age } else { 0.0 };
                return (clock.time + extra).max(0.0);
            }
        }
        self.playhead + dt
    }

    fn integrated_source_time(&self, dt: f64) -> f64 {
        // Source playback is wall-clock; program audio clock does not apply.
        self.source_monitor.as_ref().map(|s| s.time).unwrap_or(0.0) + dt
    }

    pub(crate) fn on_clock(
        &mut self,
        epoch: u64,
        now: f64,
        sample: Option<AudioClockSample>,
    ) -> Vec<PreviewCommand> {
        if epoch != self.clock_epoch || !self.playing || !now.is_finite() {
            return Vec::new();
        }
        let Some(sample) = sample else {
            return self.note_clock_missing(now);
        };
        self.clock_missing_at = None;
        if !sample.available {
            self.audio_unavailable = true;
            self.clock = None;
            return Vec::new();
        }
        if self.audio_unavailable {
            self.audio_unavailable = false;
            self.clock = None;
            return self.emit(Want {
                audio: false,
                transport: true,
                picture: false,
            });
        }
        self.clock = Some(NativeClock {
            time: sample.time.max(0.0),
            playing: sample.playing,
            received_at: now,
        });
        Vec::new()
    }

    pub(crate) fn on_clock_failed(&mut self, epoch: u64) -> Vec<PreviewCommand> {
        if epoch != self.clock_epoch {
            return Vec::new();
        }
        self.audio_unavailable = true;
        self.clock = None;
        Vec::new()
    }

    pub(crate) fn note_clock_missing(&mut self, now: f64) -> Vec<PreviewCommand> {
        let started = self.clock_missing_at.get_or_insert(now);
        if now - *started >= CLOCK_MISSING_SECS {
            self.audio_unavailable = true;
            self.clock = None;
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EditorCommand;
    use todo_core::{create_project, MediaItem, MediaKind};

    fn source_session() -> EditorSession {
        let mut project = create_project(None, None, None, None, None);
        project.media.push(MediaItem {
            id: "source-1".into(),
            name: "source.mp4".into(),
            path: Some("source.mp4".into()),
            duration: 3.0,
            has_audio: Some(false),
            width: Some(1920),
            height: Some(1080),
            kind: Some(MediaKind::Video),
            alpha: None,
            reveal: None,
            folder_id: None,
        });
        let mut session = EditorSession::new(project, "source-test");
        let _ = session.apply(EditorCommand::OpenSourceMonitor {
            media_id: "source-1".into(),
        });
        session
    }

    #[test]
    fn source_playback_restarts_from_zero_after_reaching_end() {
        let mut session = source_session();
        session.source_monitor.as_mut().unwrap().time = 3.0;

        let commands = session.apply(EditorCommand::TogglePlay).commands;

        assert!(session.playing);
        assert_eq!(session.source_monitor.as_ref().unwrap().time, 0.0);
        assert!(commands.iter().any(|command| matches!(
            command,
            PreviewCommand::Transport { time, playing: true } if *time == 0.0
        )));
    }
}
