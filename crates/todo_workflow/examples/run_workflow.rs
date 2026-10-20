//! Run an installed workflow graph end to end with offline stub services.
//!
//! ```text
//! cargo run -p todo_workflow --example run_workflow -- <extension-id> [script]
//! ```
//!
//! Stubs answer chat/image/tts/subtitles deterministically, so this checks the
//! graph wiring (triggers → script → segmentation → TTS → subtitles → emit)
//! without spending on a provider. Nodes that are still design-stage report
//! themselves as unimplemented.
use std::path::PathBuf;
use todo_production::extension::protocol::{ChatCall, ImageCall, SpeechCall, SubtitlesCall};
use todo_workflow::runner::{run, RunInput, Services};
use todo_workflow::{Origin, Registry};

struct Offline;

impl Services for Offline {
    fn chat(&mut self, call: ChatCall) -> Result<serde_json::Value, String> {
        // 分段的语意模式期待 {"segments":[{"start":n}]}；给一个能让规则接住的答复
        let prompt = call.messages.last().map(|m| m.content.clone()).unwrap_or_default();
        let segments = if prompt.contains("分段") || prompt.contains("segment") {
            serde_json::json!({ "text": r#"{"segments":[{"start":0},{"start":3}]}"# })
        } else {
            serde_json::json!({ "text": "（离线占位回复）" })
        };
        Ok(segments)
    }

    fn image(&mut self, _call: ImageCall) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "path": "out/stub.png", "mime": "image/png", "width": 1024, "height": 1024 }))
    }

    fn vision(&mut self, _prompt: String, images: Vec<PathBuf>) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "text": format!("{} 张图（离线）", images.len()) }))
    }

    fn speech(&mut self, call: SpeechCall) -> Result<serde_json::Value, String> {
        let chars: Vec<serde_json::Value> = call
            .text
            .chars()
            .enumerate()
            .map(|(index, ch)| {
                serde_json::json!({
                    "text": ch.to_string(),
                    "start": index as f64 * 0.12,
                    "end": (index + 1) as f64 * 0.12,
                })
            })
            .collect();
        let duration = chars.len() as f64 * 0.12;
        Ok(serde_json::json!({
            "file": "out/voice.wav",
            "duration": duration,
            "timeline": { "text": call.text, "duration": duration, "chars": chars },
        }))
    }

    fn subtitles(&mut self, call: SubtitlesCall) -> Result<serde_json::Value, String> {
        let cues: Vec<serde_json::Value> = call
            .script
            .split('。')
            .filter(|part| !part.trim().is_empty())
            .enumerate()
            .map(|(index, part)| {
                serde_json::json!({
                    "text": part.trim(),
                    "start": index as f64 * 2.0,
                    "end": index as f64 * 2.0 + 1.9,
                })
            })
            .collect();
        Ok(serde_json::json!({ "cues": cues }))
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = std::env::args().skip(1);
    let Some(id) = args.next() else {
        eprintln!("usage: run_workflow <extension-id> [script]");
        std::process::exit(2);
    };
    let script = args.next().unwrap_or_default();

    let registry = Registry::scan(&root.join("extensions/builtin"), &root.join("extensions/user"));
    for rejected in &registry.rejected {
        eprintln!("rejected {}: {:?}", rejected.dir.display(), rejected.problems);
    }
    let Some(extension) = registry.get(&id) else {
        eprintln!("没有扩展 {id}；可用：{:?}", registry.list().map(|e| e.manifest.id.clone()).collect::<Vec<_>>());
        std::process::exit(1);
    };
    let _ = Origin::Builtin;
    let workflow = match extension.workflow() {
        Ok(Some(workflow)) => workflow,
        Ok(None) => {
            eprintln!("{} 没有 graph.json", extension.manifest.id);
            std::process::exit(1);
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("{}: {}", problem.path, problem.message);
            }
            std::process::exit(1);
        }
    };
    println!("graph {} ({} nodes, {} edges)", workflow.extension_id, workflow.graph.nodes.len(), workflow.graph.edges.len());

    let mut services = Offline;
    let mut emitted = Vec::new();
    let report = run(
        &workflow.graph,
        &extension.dir,
        &mut services,
        &RunInput { script, at: Some(0.0) },
        &mut |items| emitted.push(items),
    );
    for node in &report.nodes {
        println!(
            "  {:<16} {:<16} {}{}",
            node.id,
            node.node_type,
            if node.ok { "ok" } else { "FAILED" },
            if node.message.is_empty() {
                String::new()
            } else {
                format!(" — {}", node.message)
            }
        );
    }
    println!("emitted {} payload(s)", emitted.len());
    match report.error {
        Some(error) => {
            println!("stopped: {error}");
            std::process::exit(1);
        }
        None => println!("done"),
    }
}
