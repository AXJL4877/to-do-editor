use crate::{tools::capture, MediaError, MediaService};
use std::{ffi::OsString, fs};

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8])
        && bytes.ends_with(&[0xff, 0xd9])
        && image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).is_ok()
}
impl MediaService {
    pub fn get_thumb(&self, stored: &str, width: u32) -> Result<Vec<u8>, MediaError> {
        let full = self.resolve(stored)?;
        let width = (width.clamp(32, 512) / 2) * 2;
        let key = self.cache.key(&full, "thumbnails", width)?;
        let destination = self.cache.path(&key, "jpg");
        self.cache.with_key(&key, || {
            if let Ok(bytes) = fs::read(&destination) {
                if is_jpeg(&bytes) {
                    return Ok(bytes);
                }
            }
            let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]
                .map(Into::into)
                .to_vec();
            args.push(full.as_os_str().into());
            args.extend(
                [
                    "-an",
                    "-sn",
                    "-vf",
                    &format!("scale={width}:-2:flags=lanczos"),
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "-c:v",
                    "mjpeg",
                    "pipe:1",
                ]
                .map(Into::into),
            );
            let bytes = capture(&self.tools.ffmpeg, args, self.timeout, 8 * 1024 * 1024)?;
            if !is_jpeg(&bytes) {
                return Err(MediaError::Invalid(
                    "thumbnail decoder produced no complete JPEG".into(),
                ));
            }
            self.cache.write(&destination, &bytes);
            Ok(bytes)
        })
    }
}
