//! Render program frames offscreen so an agent can look at its own edit.
//!
//! ```text
//! cargo run -p todo_app --example render_frame -- <project> <out-dir> 3 12 24.5
//! ```
//!
//! Prints one JSON object per frame: `{"time":..,"png":"..","width":..,"height":..}`.
use std::path::PathBuf;
use todo_app::adapters::frame::render_frames;
use todo_storage::ProjectStore;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let (Some(name), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: render_frame <project> <out-dir> <seconds>...");
        std::process::exit(2);
    };
    let times: Vec<f64> = args.filter_map(|value| value.parse().ok()).collect();
    if times.is_empty() {
        eprintln!("需要至少一个时间点（秒）");
        std::process::exit(2);
    }

    let store = ProjectStore::new(root.clone());
    let project = match store.load(&name) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("load {name}: {error}");
            std::process::exit(1);
        }
    };
    match render_frames(&root, &project, &times, &PathBuf::from(out_dir)) {
        Ok(frames) => {
            for frame in frames {
                println!(
                    "{}",
                    serde_json::json!({
                        "time": frame.time,
                        "png": frame.png.to_string_lossy(),
                        "width": frame.width,
                        "height": frame.height,
                    })
                );
            }
        }
        Err(error) => {
            eprintln!("render failed: {error}");
            std::process::exit(1);
        }
    }
}
