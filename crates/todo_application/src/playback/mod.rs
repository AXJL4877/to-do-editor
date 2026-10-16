use crate::editor::*;
use todo_core::{project_duration, seek};

impl EditorSession {
    pub(crate) fn toggle_play(&mut self) -> Vec<PreviewCommand> {
        if (self.importing || !self.can_play_at(self.picture().active)) && !self.playing {
            return Vec::new();
        }
        self.playing = !self.playing;
        self.last_tick = None;
        self.audio_unavailable = false;
        self.clock_missing_at = None;
        self.emit(Want {
            audio: false,
            transport: true,
            picture: false,
        })
    }

    pub(crate) fn scrub(&mut self, time: f64) -> Vec<PreviewCommand> {
        let was_playing = self.playing;
        self.playing = false;
        self.last_tick = None;
        let next = seek(self.project.clone(), time);
        let moved = playhead_changed(next.playhead, self.playhead);
        self.project = next;
        self.playhead = self.project.playhead;
        self.synced_playhead = self.playhead;
        self.emit(Want {
            audio: false,
            transport: was_playing || moved,
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
