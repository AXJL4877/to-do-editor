//! Export a project through the app's own timeline renderer: native offline
//! renderer for frames, FFmpeg for encode, then the same audio mix and quality
//! report the editor's 导出 button uses.
//!
//! ```text
//! cargo run -p todo_app --example export_project -- <project> [output.mp4]
//! ```
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use todo_app::adapters::export::export_timeline;
use todo_media::ExportPreset;
use todo_storage::ProjectStore;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let Some(name) = args.next() else {
        eprintln!("usage: export_project <project> [output.mp4] [--check]");
        std::process::exit(2);
    };
    let rest: Vec<String> = args.collect();
    let check_only = rest.iter().any(|value| value == "--check");
    let output = rest
        .iter()
        .find(|value| !value.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/prod/export.mp4"));

    let store = ProjectStore::new(root.clone());
    let project = match store.load(&name) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("load {name}: {error}");
            std::process::exit(1);
        }
    };
    println!(
        "project {name}: {} tracks, {} media, {} text styles",
        project.tracks.len(),
        project.media.len(),
        project.text_styles.len()
    );

    if check_only {
        println!("check ok");
        return;
    }

    // `TODO_EXPORT_PRESET=high` trades size for headroom on the PSNR gate.
    let preset = match std::env::var("TODO_EXPORT_PRESET").as_deref() {
        Ok("high") => ExportPreset::HighQuality,
        Ok("draft") => ExportPreset::Draft,
        _ => ExportPreset::Standard1080p30,
    };
    let cancel = AtomicBool::new(false);
    let mut progress = |done: usize, total: usize| {
        if done == total || done % 300 == 0 {
            println!("frames {done}/{total}");
        }
    };
    match export_timeline(&root, &project, &output, preset, &cancel, &mut progress) {
        Ok(outcome) => println!(
            "ok {} frames={} report={:?} failed_checks={:?}",
            outcome.output.display(),
            outcome.frames,
            outcome.report,
            outcome.failed_checks
        ),
        Err(error) => {
            eprintln!("export failed: {error}");
            std::process::exit(1);
        }
    }
}
