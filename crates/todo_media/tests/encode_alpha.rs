use image::{Rgba, RgbaImage};
use std::sync::{atomic::AtomicBool, Arc};
use todo_media::{encode_frames, EncodeQuality};

#[test]
fn encodes_three_frames_with_stacked_alpha_metadata() {
    let ffmpeg = std::env::var_os("TODO_FFMPEG")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../dependencies/ffmpeg/bin/ffmpeg.exe")
        });
    if !ffmpeg.is_file() {
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/encode-alpha-tests")
        .join(uuid::Uuid::new_v4().to_string());
    let frames = root.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    for index in 0..3 {
        let mut image = RgbaImage::from_pixel(8, 4, Rgba([0, 0, 0, 0]));
        for x in 1..=index + 1 {
            for y in 1..3 {
                *image.get_pixel_mut(x, y) = Rgba([220, 80, 40, 255]);
            }
        }
        image
            .save(frames.join(format!("{:06}.png", index + 1)))
            .unwrap();
    }
    let output = root.join("stacked.mp4");
    let info = encode_frames(
        &frames,
        3.0,
        true,
        EncodeQuality::Standard,
        None,
        &output,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!((info.width, info.height, info.frames), (8, 4, 3));
    let ffprobe = ffmpeg.parent().unwrap().join("ffprobe.exe");
    if ffprobe.is_file() {
        let probe = std::process::Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format_tags=todo_alpha",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "stacked");
    }
    let _ = std::fs::remove_dir_all(root);
}
