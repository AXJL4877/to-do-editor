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
