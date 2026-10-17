mod probe;
#[cfg(test)]
mod tests;
use crate::{tools::capture, MediaError, MediaService};
use serde::Deserialize;
use std::{ffi::OsString, path::Path};
use todo_core::MediaKind;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaMetadata {
    pub duration: f64,
    pub has_audio: bool,
    pub audio_channels: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub kind: MediaKind,
    pub alpha: Option<AlphaPacking>,
    pub reveal: Option<todo_core::RevealInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaPacking {
    Stacked,
    /// `todo-reveal/1`: picture and appearance times side by side.
    Reveal,
}
#[derive(Deserialize, Default)]
pub(super) struct ProbeJson {
    #[serde(default)]
    format: Format,
    #[serde(default)]
    streams: Vec<Stream>,
}
#[derive(Deserialize, Default)]
pub(super) struct Format {
    duration: Option<String>,
    #[serde(default)]
    format_name: String,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
}
#[derive(Deserialize, Default)]
pub(super) struct Stream {
    #[serde(default)]
    codec_type: String,
    width: Option<u32>,
    height: Option<u32>,
    channels: Option<u32>,
    duration: Option<String>,
    #[serde(default)]
    disposition: Disposition,
}
#[derive(Deserialize, Default)]
pub(super) struct Disposition {
    #[serde(default)]
    attached_pic: u32,
}

impl MediaService {
    pub fn probe_media(&self, stored: &str) -> Result<MediaMetadata, MediaError> {
        self.probe_path(&self.resolve(stored)?)
    }
    pub(crate) fn probe_path(&self, full: &Path) -> Result<MediaMetadata, MediaError> {
        if full
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".reveal.png"))
        {
            // A reveal atlas: final picture beside its appearance times.
            let (meta, _, _) = crate::read_reveal(full)?;
            let hand = meta.hand.as_ref().and_then(|hand| {
                let image = full.parent()?.join(&hand.file);
                let (width, height) = crate::image_size(&image).ok()?;
                Some(todo_core::RevealHand {
                    file: hand.file.clone(),
                    width,
                    height,
                    anchor: hand.anchor,
                    scale: hand.height,
                    path: hand.path.clone(),
                })
            });
            return Ok(MediaMetadata {
                duration: meta.duration.max(0.04),
                has_audio: false,
                audio_channels: 0,
                width: Some(meta.width),
                height: Some(meta.height),
                kind: MediaKind::Image,
                alpha: Some(AlphaPacking::Reveal),
                reveal: Some(todo_core::RevealInfo {
                    soft_max: meta.soft_max,
                    hand,
                }),
            });
        }
        let mut args: Vec<OsString> = [
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ]
        .map(Into::into)
        .to_vec();
        args.push(full.as_os_str().into());
        let bytes = capture(&self.tools.ffprobe, args, self.timeout, 4 * 1024 * 1024)?;
        probe::parse(full, serde_json::from_slice(&bytes)?)
    }
}
