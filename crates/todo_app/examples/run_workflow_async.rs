//! 增量驱动验证：不阻塞地跑完一条工作流（与 App 面板同一套逻辑）。
//!
//! ```text
//! cargo run -p todo_app --example run_workflow_async -- <extension-id> "文案"
//! ```
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use todo_app::adapters::production::bridge::ServiceBridge;
use todo_ai::{AiConfig, AiRouter, CapabilityBindings, ModelBinding, Secrets, ServiceSettings};
use todo_workflow::workflow::protocol::ServiceCall;
use todo_workflow::workflow::HostDefaults;
use todo_production::tasks::MultimodalRequest;
use todo_production::{Capability, Provider, TaskKey, TaskRequest};
use todo_workflow::pool::PoolOptions;
use todo_workflow::runner::{RequestedCall, RunInput, Step, WorkflowJob};
use todo_workflow::runtime::RuntimePaths;
use todo_workflow::{Registry, RunId, WorkerPool};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = std::env::args().skip(1);
    let Some(id) = args.next() else {
        eprintln!("usage: run_workflow_async <extension-id> [script]");
        std::process::exit(2);
    };
    let script = args.next().unwrap_or_default();

    let config = match AiConfig::load(&root) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("config: {error}");
            std::process::exit(1);
        }
    };
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &todo_ai::secrets::process_environment,
    };
    let mut keys = std::collections::HashMap::new();
    for provider in [Provider::Huniu, Provider::Volc] {
        if let Ok(Some((key, _))) = secrets.resolve(provider) {
            keys.insert(provider, key);
        }
    }
    let language = config.language_base_url.as_ref().and_then(|base| {
        secrets.language().ok().flatten().map(|(key, _)| todo_ai::LanguageEndpoint {
            base_url: base.clone(),
            key,
            model: config
                .language_model
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| "deepseek-flash".into()),
        })
    });
    let language_model = language
        .as_ref()
        .map(|endpoint| endpoint.model.clone())
        .unwrap_or_else(|| config.huniu.language_model.clone());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let workdir = root.join("cache/workflow-runs").join(format!("{id}-{stamp}"));
    std::fs::create_dir_all(&workdir).unwrap();
    let model = |capability: Capability| match capability {
        Capability::Language => language_model.clone(),
        Capability::Image => config.huniu.image_model.clone(),
        Capability::Video => config.huniu.video_model.clone(),
        Capability::Multimodal => config.huniu.multimodal_model.clone(),
        Capability::Audio => config.volc.audio_model.clone(),
    };
    let settings = ServiceSettings {
        huniu_base_url: config.huniu.base_url.clone(),
        language,
        volc_base_url: config.volc.base_url.clone(),
        huniu_strategy: config.huniu.strategy.clone(),
        keys,
        bindings: std::collections::HashMap::from([
            (Capability::Language, Provider::Huniu),
            (Capability::Image, Provider::Huniu),
            (Capability::Video, Provider::Huniu),
            (Capability::Multimodal, Provider::Huniu),
            (Capability::Audio, Provider::Volc),
        ]),
        models: std::collections::HashMap::from([
            (Capability::Language, model(Capability::Language)),
            (Capability::Image, model(Capability::Image)),
            (Capability::Video, model(Capability::Video)),
            (Capability::Multimodal, model(Capability::Multimodal)),
            (Capability::Audio, model(Capability::Audio)),
        ]),
        // TODO_WORKFLOW_JOURNAL=global 用 App 的那份日志（复现缓存/重试行为）
        journal_dir: match std::env::var("TODO_WORKFLOW_JOURNAL").as_deref() {
            Ok("global") => todo_ai::default_paths(&root).0,
            _ => workdir.join("journal"),
        },
        artifacts_root: workdir.join("blobs"),
        proxy: config.proxy,
        limits: config.limits.clone(),
        retry: todo_ai::net::RetryPolicy::default(),
        breaker: todo_ai::net::BreakerPolicy::default(),
    };
    let bindings = CapabilityBindings {
        models: std::collections::HashMap::from([
            (
                Capability::Language,
                ModelBinding { model: model(Capability::Language), provider: Provider::Huniu },
            ),
            (
                Capability::Image,
                ModelBinding { model: model(Capability::Image), provider: Provider::Huniu },
            ),
            (
                Capability::Multimodal,
                ModelBinding { model: model(Capability::Multimodal), provider: Provider::Huniu },
            ),
            (
                Capability::Audio,
                ModelBinding { model: model(Capability::Audio), provider: Provider::Volc },
            ),
        ]),
    };
    let (ai, _) = match AiRouter::start(settings, bindings) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("ai router: {error}");
            std::process::exit(1);
        }
    };
    let (events, _inbox) = mpsc::channel();
    let pool = WorkerPool::start(
        PoolOptions { runtime: RuntimePaths::new(root.clone()), ..PoolOptions::default() },
        events,
    );
    let mut bridge = ServiceBridge::new(
        HostDefaults { audio_model: config.volc.audio_model.clone(), speaker: config.volc.speaker.clone(), sample_rate: 24_000 },
        &root,
        pool.handle(),
    );
    let run_id: RunId = stamp as RunId;
    bridge.register_run(run_id, workdir.clone());

    let registry = Registry::scan(&root.join("extensions/builtin"), &root.join("extensions/user"));
    let Some(extension) = registry.get(&id) else {
        eprintln!("没有扩展 {id}");
        std::process::exit(1);
    };
    let workflow = match extension.workflow() {
        Ok(Some(workflow)) => workflow,
        _ => {
            eprintln!("{id} 没有可用的 graph.json");
            std::process::exit(1);
        }
    };
    let mut job = WorkflowJob::new(&workflow.graph, &extension.dir, RunInput { script, at: Some(0.0) });

    // ---- 与 App 面板一致的驱动：不阻塞，事件回来再继续 ----
    // call_id → (node_id, via_router, started)
    let mut calls: std::collections::HashMap<String, (String, bool, Instant)> = Default::default();
    let mut vision: std::collections::HashMap<TaskKey, String> = Default::default();
    let mut results: std::collections::HashMap<String, Result<serde_json::Value, String>> =
        Default::default();
    let mut seq = 0;
    let started = Instant::now();
    let mut tick = 0usize;
    loop {
        while let Some(event) = ai.poll() {
            let key = match &event {
                todo_ai::AiEvent::Done { key, .. }
                | todo_ai::AiEvent::Failed { key, .. }
                | todo_ai::AiEvent::Cancelled { key } => Some(key.clone()),
                _ => None,
            };
            if let Some(key) = key {
                if let Some(call_id) = vision.remove(&key) {
                    let value = match &event {
                        todo_ai::AiEvent::Done { output, .. } => Ok(output
                            .as_ref()
                            .and_then(|v| v.get("text"))
                            .and_then(serde_json::Value::as_str)
                            .map(|t| serde_json::json!({ "text": t }))
                            .unwrap_or_else(|| serde_json::json!({ "text": "" }))),
                        todo_ai::AiEvent::Failed { message, .. } => Err(message.clone()),
                        _ => Err("已取消".into()),
                    };
                    results.insert(call_id, value);
                }
            }
            bridge.on_ai_event(&ai, &event);
        }
        let waiting: Vec<(String, String, Result<serde_json::Value, String>)> = calls
            .iter()
            .filter_map(|(call_id, (node, via_router, _))| {
                let result = if *via_router {
                    results.remove(call_id)
                } else {
                    bridge.take_direct(call_id)
                }?;
                Some((call_id.clone(), node.clone(), result))
            })
            .collect();
        for (call_id, node, result) in waiting {
            calls.remove(&call_id);
            job.resume(&node, result);
        }
        if !waiting.is_empty() {
            continue;
        }
        match job.advance() {
            Step::Waiting => {}
            Step::Finished => break,
            Step::Calls(batch) => {
                for (node, call) in batch {
                    seq += 1;
                    let call_id = format!("wf-{run_id}-{node}-{seq}");
                    let via_router = match call {
                        RequestedCall::Chat(c) => {
                            bridge.call_async(&ai, run_id, call_id.clone(), ServiceCall::Chat(c));
                            false
                        }
                        RequestedCall::Image(c) => {
                            bridge.call_async(&ai, run_id, call_id.clone(), ServiceCall::Image(c));
                            false
                        }
                        RequestedCall::Speech(c) => {
                            bridge.call_async(
                                &ai,
                                run_id,
                                call_id.clone(),
                                ServiceCall::Speech(c),
                            );
                            false
                        }
                        RequestedCall::Subtitles(c) => {
                            bridge.call_async(
                                &ai,
                                run_id,
                                call_id.clone(),
                                ServiceCall::Subtitles(c),
                            );
                            false
                        }
                        RequestedCall::Vision { prompt, images } => {
                            let media: Vec<String> = images
                                .iter()
                                .map(|p| p.to_string_lossy().into_owned())
                                .collect();
                            let request = MultimodalRequest {
                                model: String::new(),
                                prompt,
                                digests: todo_ai::providers::vision::media_digests(&media),
                                media,
                                json: false,
                            };
                            let key = ai.submit(TaskRequest::Multimodal(request));
                            vision.insert(key, call_id.clone());
                            true
                        }
                        RequestedCall::TimelineStrip { .. } => {
                            eprintln!("timeline_strip 需桌面宿主，本示例跳过");
                            false
                        }
                        RequestedCall::WaitPlacement => {
                            eprintln!("placement_ready 需桌面宿主，本示例跳过");
                            false
                        }
                    };
                    calls.insert(call_id, (node, via_router, Instant::now()));
                }
            }
        }
        tick += 1;
        if tick % 10 == 0 {
            let (done, total) = job.progress();
            println!("  … {done}/{total}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let report = job.report();
    for node in &report.nodes {
        println!(
            "  {:<14} {:<16} {:<6} {:.2}s",
            node.id,
            node.node_type,
            if node.ok { "ok" } else { "FAILED" },
            node.duration_ms as f64 / 1000.0
        );
    }
    for node in &report.nodes {
        for line in &node.logs {
            println!("      {line}");
        }
    }
    println!(
        "{} nodes, {:.1}s (非阻塞)",
        report.nodes.len(),
        started.elapsed().as_secs_f64()
    );
    if let Some(error) = &report.error {
        println!("stopped: {error}");
        std::process::exit(1);
    }
    println!("done");
}
