#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPreset {
    Standard1080p30,
    HighQuality,
    Vertical1080p,
    Draft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetArgs {
    pub video: Vec<String>,
    pub audio: Vec<String>,
}

pub fn args(preset: ExportPreset) -> PresetArgs {
    let (crf, preset_name, audio_bitrate) = match preset {
        ExportPreset::Standard1080p30 => ("18", "slow", "192k"),
        ExportPreset::HighQuality => ("14", "slower", "256k"),
        ExportPreset::Vertical1080p => ("18", "slow", "192k"),
        ExportPreset::Draft => ("23", "veryfast", "128k"),
    };
    let mut video = vec![
        "-c:v",
        "libx264",
        "-preset",
        preset_name,
        "-crf",
        crf,
        "-pix_fmt",
        "yuv420p",
        "-colorspace",
        "bt709",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-movflags",
        "+faststart",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if matches!(
        preset,
        ExportPreset::Standard1080p30 | ExportPreset::Vertical1080p
    ) {
        video.extend(
            ["-r", "30", "-g", "60", "-color_range", "tv"]
                .into_iter()
                .map(String::from),
        );
    } else if matches!(preset, ExportPreset::HighQuality) {
        video.extend(
            ["-g", "60", "-color_range", "tv"]
                .into_iter()
                .map(String::from),
        );
    }
    PresetArgs {
        video,
        audio: vec![
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            audio_bitrate.into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_and_vertical_use_two_second_gop_and_bt709() {
        for preset in [ExportPreset::Standard1080p30, ExportPreset::Vertical1080p] {
            let args = args(preset);
            assert!(args
                .video
                .windows(2)
                .any(|pair| pair[0] == "-g" && pair[1] == "60"));
            assert!(args
                .video
                .windows(2)
                .any(|pair| pair[0] == "-color_range" && pair[1] == "tv"));
            assert!(args
                .video
                .windows(2)
                .any(|pair| pair[0] == "-colorspace" && pair[1] == "bt709"));
        }
    }
}
