use std::path::PathBuf;
use todo_media::{inspect_quality, write_report, QualityTargets};

#[test]
fn quality_report_captures_ffprobe_stream_facts() {
    let ffmpeg = std::env::var_os("TODO_FFMPEG")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../dependencies/ffmpeg/bin/ffmpeg.exe")
        });
    let ffprobe = std::env::var_os("TODO_FFPROBE")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| ffmpeg.parent().unwrap().join("ffprobe.exe"));
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/quality-tests");
    std::fs::create_dir_all(&root).unwrap();
    let output = root.join("sample.mp4");
    let status = std::process::Command::new(&ffmpeg)
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=8x8:d=1",
            "-r",
            "3",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ])
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let report = inspect_quality(
        &ffprobe,
        &output,
        Some(QualityTargets {
            fps: 3.0,
            duration: 1.0,
            frame_tolerance: 1.0 / 3.0,
        }),
    )
    .unwrap();
    assert_eq!(
        report
            .video
            .as_ref()
            .and_then(|video| video.codec.as_deref()),
        Some("h264")
    );
    assert!(report.checks.iter().all(|check| check.passed));
    let report_path = root.join("sample.mp4.report.json");
    write_report(&report_path, &report).unwrap();
    assert!(report_path.is_file());
    let _ = std::fs::remove_dir_all(root);
}
