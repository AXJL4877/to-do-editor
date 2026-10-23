//! Capture the open project's program monitor every N absolute frames and
//! stitch a contact sheet.
//!
//! ```text
//! cargo run -p todo_app --example timeline_strip -- <project> <out-dir> [everyFrames=30] [content|workspace] [maxFrames=24]
//! ```
use std::path::PathBuf;
use todo_app::adapters::frame::{render_timeline_strip, StripRange};
use todo_storage::ProjectStore;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let (Some(name), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!(
            "usage: timeline_strip <project> <out-dir> [everyFrames=30] [content|workspace] [maxFrames=24]"
        );
        std::process::exit(2);
    };
    let every: u32 = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .max(1);
    let range = StripRange::parse(&args.next().unwrap_or_else(|| "content".into()));
    let max_frames: u32 = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
        .clamp(1, 64);

    let store = ProjectStore::new(root.clone());
    let project = match store.load(&name) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("load {name}: {error}");
            std::process::exit(1);
        }
    };
    match render_timeline_strip(
        &root,
        &project,
        &PathBuf::from(out_dir),
        every,
        max_frames,
        160,
        range,
    ) {
        Ok(strip) => {
            println!(
                "{}",
                serde_json::json!({
                    "path": strip.path.to_string_lossy(),
                    "width": strip.width,
                    "height": strip.height,
                    "fps": strip.fps,
                    "everyFrames": strip.every_frames,
                    "count": strip.frames.len(),
                    "times": strip.frames.iter().map(|f| f.time).collect::<Vec<_>>(),
                })
            );
        }
        Err(error) => {
            eprintln!("strip failed: {error}");
            std::process::exit(1);
        }
    }
}
