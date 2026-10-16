use super::*;

pub(super) fn parse(full: &Path, json: ProbeJson) -> Result<MediaMetadata, MediaError> {
    let video = json
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video" && stream.disposition.attached_pic == 0);
    let audio = json
        .streams
        .iter()
        .find(|stream| stream.codec_type == "audio");
    let extension = full
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let still = video
        .is_some_and(|video| video.width.unwrap_or(0) > 0 && video.height.unwrap_or(0) > 0)
        && (matches!(
            extension.as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp" | "gif" | "tif" | "tiff"
        ) || json.format.format_name.split(',').any(|format| {
            matches!(
                format,
                "image2"
                    | "image2pipe"
                    | "png_pipe"
                    | "jpeg_pipe"
                    | "webp_pipe"
                    | "bmp_pipe"
                    | "tiff_pipe"
                    | "png"
                    | "webp"
            )
        }));
    let kind = if still {
        MediaKind::Image
    } else if video.is_some() {
        MediaKind::Video
    } else if audio.is_some() {
        MediaKind::Audio
    } else {
        return Err(MediaError::Invalid("no media streams".into()));
    };
    let duration = if still {
        todo_core::DEFAULT_STILL_DURATION_SECS
    } else {
        json.format
            .duration
            .as_deref()
            .and_then(|duration| duration.parse::<f64>().ok())
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .or_else(|| {
                json.streams
                    .iter()
                    .filter_map(|stream| stream.duration.as_deref()?.parse::<f64>().ok())
                    .filter(|duration| duration.is_finite() && *duration > 0.0)
                    .max_by(f64::total_cmp)
            })
            .ok_or_else(|| MediaError::Invalid("duration missing".into()))?
    };
    let channels = audio.map_or(0, |stream| stream.channels.unwrap_or(1));
    if audio.is_some() && !(1..=256).contains(&channels) {
        return Err(MediaError::Invalid("invalid audio channel count".into()));
    }
    Ok(MediaMetadata {
        duration,
        kind,
        has_audio: audio.is_some(),
        audio_channels: channels,
        width: video.and_then(|stream| stream.width),
        height: video.and_then(|stream| stream.height),
    })
}
