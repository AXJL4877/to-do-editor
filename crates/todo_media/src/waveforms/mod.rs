mod pcm;
#[cfg(test)]
mod tests;
use crate::{tools::execute, MediaError, MediaService};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waveform {
    pub duration: f64,
    pub has_audio: bool,
    pub peaks: Vec<f64>,
}
impl MediaService {
    pub fn get_peaks(&self, stored: &str, bins: u32) -> Result<Waveform, MediaError> {
        let full = self.resolve(stored)?;
        let bins = bins.clamp(1, 16_384);
        let key = self.cache.key(&full, "waveforms", bins)?;
        let destination = self.cache.path(&key, "json");
        self.cache.with_key(&key, || {
            if let Some(waveform) = fs::read(&destination)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Waveform>(&bytes).ok())
            {
                if waveform.duration.is_finite()
                    && waveform.duration > 0.0
                    && waveform.peaks.len() == if waveform.has_audio { bins as usize } else { 0 }
                    && waveform
                        .peaks
                        .iter()
                        .all(|peak| peak.is_finite() && (0.0..=1.0).contains(peak))
                {
                    return Ok(waveform);
                }
            }
            let metadata = self.probe_path(&full)?;
            let peaks = if metadata.has_audio {
                let mut args: Vec<OsString> =
                    ["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]
                        .map(Into::into)
                        .to_vec();
                args.push(full.as_os_str().into());
                args.extend(
                    [
                        "-map",
                        "0:a:0",
                        "-vn",
                        "-sn",
                        "-dn",
                        "-af",
                        "aresample=48000:async=1:first_pts=0",
                        "-t",
                        &metadata.duration.to_string(),
                        "-c:a",
                        "pcm_f32le",
                        "-f",
                        "f32le",
                        "pipe:1",
                    ]
                    .map(Into::into),
                );
                execute(&self.tools.ffmpeg, args, self.timeout, move |stdout| {
                    pcm::decode(
                        stdout,
                        metadata.duration,
                        metadata.audio_channels,
                        bins as usize,
                    )
                })?
            } else {
                Vec::new()
            };
            let waveform = Waveform {
                duration: metadata.duration,
                has_audio: metadata.has_audio,
                peaks,
            };
            if let Ok(bytes) = serde_json::to_vec(&waveform) {
                self.cache.write(&destination, &bytes);
            }
            Ok(waveform)
        })
    }
}
