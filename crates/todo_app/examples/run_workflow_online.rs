//! Run an installed workflow graph with the **real** services (no stubs).
//!
//! ```text
//! cargo run -p todo_app --example run_workflow_online -- <extension-id> "文案"
//! ```
//!
//! Builds the same AI router and service bridge the app uses (config/ai.json +
//! Windows credentials), then drives `todo_workflow::runner` through
//! `HostServices`. Paid calls: each run spends on chat / image / speech.
use std::path::PathBuf;
use std::sync::mpsc;
use todo_app::adapters::production::bridge::ServiceBridge;
use todo_app::adapters::workflow::HostServices;
use todo_ai::{
    AiConfig, AiRouter, CapabilityBindings, ModelBinding, ServiceSettings,
};
use todo_production::extension::HostDefaults;
use todo_production::{Capability, Provider};
use todo_workflow::runner::{run, RunInput};
use todo_workflow::{Registry, RunId, WorkerPool};
use todo_workflow::runtime::RuntimePaths;
use todo_workflow::pool::PoolOptions;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = std::env::args().skip(1);
    let Some(id) = args.next() else {
        eprintln!("usage: run_workflow_online <extension-id> [script]");
        std::process::exit(2);
    };
    let script = args.next().unwrap_or_default();

    // ---- AI router (same wiring as ProductionHost) -------------------------
    let config = match AiConfig::load(&root) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("config: {error}");
            std::process::exit(1);
        }
    };
    let store = todo_ai::secrets::default_store();
    let secrets = todo_ai::Secrets {
        store: store.as_ref(),
        environment: &todo_ai::secrets::process_environment,
    };
    let mut keys = std::collections::HashMap::new();
    for provider in [Provider::Huniu, Provider::Volc] {
        match secrets.resolve(provider) {
            Ok(Some((key, source))) => {
                println!("{provider:?} key: {source:?} {}", key.masked());
                keys.insert(provider, key);
            }
            Ok(None) => println!("{provider:?} key: 缺失"),
            Err(error) => println!("{provider:?} key: {error}"),
        }
    }
    let language = config.language_base_url.as_ref().and_then(|base| {
        secrets.language().ok().flatten().map(|(key, _)| todo_ai::LanguageEndpoint {
            base_url: base.clone(),
            key,
        })
    });
    println!(
        "language: {:?} model {:?}",
        config.language_base_url, config.language_model
    );
    // 调试：日志/产物都放在本次运行目录，不读也不写全局 cache
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let workdir = root
        .join("cache/workflow-runs")
        .join(format!("{id}-{stamp}"));
    std::fs::create_dir_all(&workdir).unwrap();
    let _ = todo_ai::default_paths(&root);
    let journal_dir = workdir.join("journal");
    let artifacts_root = workdir.join("blobs");
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
            (Capability::Language, config.language_model.clone().unwrap_or_else(|| config.huniu.language_model.clone())),
            (Capability::Image, config.huniu.image_model.clone()),
            (Capability::Video, config.huniu.video_model.clone()),
            (Capability::Multimodal, "deepseek-flash".to_string()),
            (Capability::Audio, config.volc.audio_model.clone()),
        ]),
        journal_dir,
        artifacts_root,
        proxy: config.proxy,
        limits: config.limits.clone(),
        retry: todo_ai::net::RetryPolicy::default(),
        breaker: todo_ai::net::BreakerPolicy::default(),
    };
    let bindings = CapabilityBindings {
        models: std::collections::HashMap::from([
            (
                Capability::Language,
                ModelBinding { model: config.language_model.clone().unwrap_or_else(|| config.huniu.language_model.clone()), provider: Provider::Huniu },
            ),
            (
                Capability::Image,
                ModelBinding { model: config.huniu.image_model.clone(), provider: Provider::Huniu },
            ),
            (
                Capability::Video,
                ModelBinding { model: config.huniu.video_model.clone(), provider: Provider::Huniu },
            ),
            (
                Capability::Multimodal,
                ModelBinding { model: "deepseek-flash".into(), provider: Provider::Huniu },
            ),
            (
                Capability::Audio,
                ModelBinding { model: config.volc.audio_model.clone(), provider: Provider::Volc },
            ),
        ]),
    };
    let (ai, recovered) = match AiRouter::start(settings, bindings) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("ai router: {error}");
            std::process::exit(1);
        }
    };
    if !recovered.is_empty() {
        println!("recovered {} task(s) from the journal", recovered.len());
    }

    // ---- worker pool + service bridge --------------------------------------
    let (events, _inbox) = mpsc::channel();
    let pool = WorkerPool::start(
        PoolOptions {
            runtime: RuntimePaths::new(root.clone()),
            ..PoolOptions::default()
        },
        events,
    );
    let mut bridge = ServiceBridge::new(
        HostDefaults {
            audio_model: config.volc.audio_model.clone(),
            sample_rate: 24_000,
        },
        &root,
        pool.handle(),
    );

    // ---- graph -------------------------------------------------------------
    let registry = Registry::scan(&root.join("extensions/builtin"), &root.join("extensions/user"));
    for rejected in &registry.rejected {
        eprintln!("rejected {}: {:?}", rejected.dir.display(), rejected.problems);
    }
    let Some(extension) = registry.get(&id) else {
        eprintln!("没有扩展 {id}");
        std::process::exit(1);
    };
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
    let run_id: RunId = stamp as RunId;
    bridge.register_run(run_id, workdir.clone());
    println!("graph {} → {}", workflow.extension_id, workdir.display());

    let mut services = HostServices::new(&mut bridge, &ai, run_id);
    let started = std::time::Instant::now();
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
            "  {:<14} {:<16} {}{}",
            node.id,
            node.node_type,
            if node.ok { "ok" } else { "FAILED" },
            if node.message.is_empty() {
                String::new()
            } else {
                format!(" — {}", node.message)
            }
        );
        if let Some(preview) = &node.preview {
            let short: String = preview.chars().take(160).collect();
            println!("       └ {}", short.replace('\n', " "));
        }
        if let Some(image) = &node.image {
            println!("       └ image {image}");
        }
    }
    println!(
        "emitted {} payload(s), {:.1}s",
        emitted.len(),
        started.elapsed().as_secs_f64()
    );
    if let Some(error) = report.error {
        println!("stopped: {error}");
        std::process::exit(1);
    }
    println!("done");
}
