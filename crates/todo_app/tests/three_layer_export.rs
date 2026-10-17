//! A small export acceptance fixture: video + reveal + subtitle image and two
//! independent audio clips. It exercises the same native renderer and quality
//! report used by the application, rather than only testing the FFmpeg wrapper.

use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use todo_core::{
    create_project, normalize_clip, AlphaPacking, MediaItem, MediaKind, Project, Track, TrackKind,
};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn wav(path: &Path, rate: u32, seconds: f64, frequency: f64) {
    let count = (rate as f64 * seconds) as usize;
    let mut samples = Vec::with_capacity(count * 2);
    for index in 0..count {
        let value = ((index as f64 * frequency * std::f64::consts::TAU / rate as f64).sin()
            * 3500.0) as i16;
        samples.extend_from_slice(&value.to_le_bytes());
    }
    let data_len = samples.len() as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.extend_from_slice(&samples);
    std::fs::write(path, bytes).unwrap();
}

fn image(path: &Path, width: u32, height: u32, color: [u8; 4]) {
    let image = RgbaImage::from_pixel(width, height, Rgba(color));
    image.save(path).unwrap();
}

fn media(id: &str, name: &str, path: &str, kind: MediaKind, duration: f64) -> MediaItem {
    MediaItem {
        id: id.into(),
        name: name.into(),
        path: Some(path.into()),
        duration,
        has_audio: Some(kind == MediaKind::Audio),
        width: (kind != MediaKind::Audio).then_some(64),
        height: (kind != MediaKind::Audio).then_some(32),
        kind: Some(kind),
        alpha: None,
        reveal: None,
        folder_id: None,
    }
}

fn track(id: &str, kind: TrackKind, clips: Vec<todo_core::Clip>) -> Track {
    Track {
        id: id.into(),
        name: id.into(),
        kind,
        muted: false,
        locked: false,
        clips,
    }
}

#[test]
fn three_layer_export_passes_quality_gate() {
    let root = repo().join("target/three-layer-export");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("media")).unwrap();
    let ffmpeg = repo().join("dependencies/ffmpeg/bin/ffmpeg.exe");
    let ffprobe = repo().join("dependencies/ffmpeg/bin/ffprobe.exe");
    let host = repo().join("native/build/bin/preview_host.exe");
    if !(ffmpeg.is_file() && ffprobe.is_file() && host.is_file()) {
        eprintln!("skipped: build native host and bundled FFmpeg first");
        return;
    }
    std::env::set_var("TODO_FFMPEG", &ffmpeg);
    std::env::set_var("TODO_FFPROBE", &ffprobe);
    std::env::set_var("TODO_PREVIEW_HOST", &host);

    let video = root.join("media/video.mp4");
    let ffmpeg_result = Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=64x32:r=30:d=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(&video)
        .status()
        .unwrap();
    assert!(ffmpeg_result.success());
    image(
        &root.join("media/subtitle.png"),
        64,
        8,
        [250, 250, 250, 255],
    );
    let color = RgbaImage::from_pixel(32, 16, Rgba([250, 80, 40, 255]));
    let mut timing = RgbaImage::from_pixel(32, 16, Rgba([0, 0, 0, 0]));
    for (index, pixel) in timing.pixels_mut().enumerate() {
        let visible_at = (index as u32 * 65_535 / (32 * 16)) as u16;
        pixel.0 = [(visible_at >> 8) as u8, visible_at as u8, 0, 255];
    }
    todo_media::write_reveal(
        &root.join("media/reveal.reveal.png"),
        &color,
        &timing,
        todo_media::RevealMeta {
            spec: todo_media::REVEAL_SPEC.into(),
            width: 32,
            height: 16,
            duration: 1.0,
            soft_max: 0.0,
            hand: None,
        },
    )
    .unwrap();
    wav(&root.join("media/voice.wav"), 24_000, 1.0, 220.0);
    wav(&root.join("media/music.wav"), 24_000, 1.0, 330.0);

    let mut reveal = media(
        "reveal",
        "reveal.reveal.png",
        "media/reveal.reveal.png",
        MediaKind::Image,
        1.0,
    );
    reveal.alpha = Some(AlphaPacking::Reveal);
    reveal.reveal = Some(todo_core::RevealInfo {
        soft_max: 0.0,
        hand: None,
    });
    let project = Project {
        media: vec![
            media(
                "video",
                "video.mp4",
                "media/video.mp4",
                MediaKind::Video,
                1.0,
            ),
            reveal,
            media(
                "subtitle",
                "subtitle.png",
                "media/subtitle.png",
                MediaKind::Image,
                1.0,
            ),
            media(
                "voice",
                "voice.wav",
                "media/voice.wav",
                MediaKind::Audio,
                1.0,
            ),
            media(
                "music",
                "music.wav",
                "media/music.wav",
                MediaKind::Audio,
                1.0,
            ),
        ],
        tracks: vec![
            track(
                "video",
                TrackKind::Video,
                vec![normalize_clip(
                    "video-clip".into(),
                    "video".into(),
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )],
            ),
            track(
                "reveal",
                TrackKind::Video,
                vec![normalize_clip(
                    "reveal-clip".into(),
                    "reveal".into(),
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )],
            ),
            track(
                "subtitle",
                TrackKind::Video,
                vec![normalize_clip(
                    "subtitle-clip".into(),
                    "subtitle".into(),
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )],
            ),
            track(
                "voice",
                TrackKind::Audio,
                vec![normalize_clip(
                    "voice-clip".into(),
                    "voice".into(),
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )],
            ),
            track(
                "music",
                TrackKind::Sfx,
                vec![normalize_clip(
                    "music-clip".into(),
                    "music".into(),
                    0.0,
                    1.0,
                    0.0,
                    1.0,
                )],
            ),
        ],
        ..create_project(Some(30.0), Some(0.0), Some(1.0), None, None)
    };
    let output = root.join("export/three-layer.mp4");
    let outcome = todo_app::adapters::export::export_timeline(
        &root,
        &project,
        &output,
        todo_media::ExportPreset::Draft,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert!(output.is_file());
    assert!(
        outcome.failed_checks.is_empty(),
        "{:?}",
        outcome.failed_checks
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&outcome.report).unwrap()).unwrap();
    assert!(
        report["psnrDb"].as_f64().is_some_and(|psnr| psnr >= 45.0),
        "{report}"
    );
    assert!(report["audio"].is_object(), "{report}");
    let _ = std::fs::remove_dir_all(root);
}
