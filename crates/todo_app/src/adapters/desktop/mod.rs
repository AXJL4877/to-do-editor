pub mod window;
use std::path::PathBuf;
pub fn media_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new()
        .add_filter(
            "媒体",
            &[
                "mp4", "mov", "mkv", "avi", "webm", "mp3", "wav", "flac", "aac", "m4a", "ogg",
                "png", "jpg", "jpeg", "bmp", "webp",
            ],
        )
        .pick_files()
}
pub fn project_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("工程 JSON", &["json"])
        .pick_file()
}
