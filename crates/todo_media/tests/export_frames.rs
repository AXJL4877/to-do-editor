use image::{Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
use todo_media::{export_frames, ExportPreset, ExportRequest, MediaTools};

#[test]
fn exports_png_sequence_and_writes_quality_report() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/export-tests");
    let frames = root.join("frames");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&frames).unwrap();
    for index in 0..3 {
        let image = RgbaImage::from_pixel(8, 8, Rgba([220, index * 20, 40, 255]));
        image
            .save(frames.join(format!("{:06}.png", index + 1)))
            .unwrap();
    }
    let tools = MediaTools::discover(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    if !tools.ffmpeg.is_file() || !tools.ffprobe.is_file() {
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let output = root.join("result.mp4");
    let result = export_frames(
        &tools,
        &ExportRequest {
            frames,
            fps: 30.0,
            audio: None,
            output: output.clone(),
            preset: ExportPreset::Standard1080p30,
        },
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.frames, 3);
    assert!(result.output.is_file());
    assert!(result.report_path.is_file());
    assert!(result.report.checks.iter().all(|check| check.passed));
    assert!(result.report.psnr_db.is_some_and(|value| value > 40.0));
    let _ = std::fs::remove_dir_all(root);
}
