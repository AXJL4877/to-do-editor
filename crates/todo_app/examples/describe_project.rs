//! Dump the AI-facing project view: every element with timing, placement and
//! its pixel rectangle on the canvas.
//!
//! ```text
//! cargo run -p todo_app --example describe_project -- <project> [at-seconds]
//! ```
use std::path::PathBuf;
use todo_application::control::{project_view, ControlCommand, ControlResult};
use todo_storage::ProjectStore;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::env::var_os("TODO_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let Some(name) = args.next() else {
        eprintln!("usage: describe_project <project> [at-seconds]");
        std::process::exit(2);
    };
    let at = args.next().and_then(|value| value.parse::<f64>().ok());

    let store = ProjectStore::new(root);
    let project = match store.load(&name) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("load {name}: {error}");
            std::process::exit(1);
        }
    };
    // The same command the control protocol exposes to agents.
    let _ = ControlCommand::Describe { at };
    let view = project_view(&project, at);
    match serde_json::to_string_pretty(&ControlResult::Document {
        document: Box::new(view),
    }) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("serialize: {error}");
            std::process::exit(1);
        }
    }
}
