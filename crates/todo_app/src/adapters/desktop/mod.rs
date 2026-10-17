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

pub fn export_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MP4 视频", &["mp4"])
        .set_file_name("export.mp4")
        .save_file()
}

pub fn image_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp"])
        .pick_file()
}
