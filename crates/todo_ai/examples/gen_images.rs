//! Generate a batch of images through the real service (spends money).
//! `cargo run -p todo_ai --example gen_images -- <list.json> <out dir>`
//!
//! `list.json` is `[{ "id": "krabs-explain", "prompt": "…", "size": "1024x1536",
//! "transparent": true, "quality": "high", "variant": 0, "references": [] }]`. Each finished
//! image is copied to `<out>/<id>.<ext>`. The journal lives in
//! `target/gen-images/`, so a rerun only pays for what is missing or changed.
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use todo_ai::net::{BreakerPolicy, RetryPolicy};
use todo_ai::secrets::{process_environment, Secrets};
use todo_ai::{AiConfig, AiEvent, AiService, ServiceSettings};
use todo_production::*;

#[derive(Deserialize)]
struct Job {
    id: String,
    prompt: String,
    #[serde(default = "square")]
    size: String,
    #[serde(default)]
    transparent: bool,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    variant: u32,
    /// Reference pictures (paths) the result must stay consistent with.
    #[serde(default)]
    references: Vec<String>,
}

fn square() -> String {
    "1024x1024".into()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let list = args.next().expect("list.json");
    let out = std::path::PathBuf::from(args.next().expect("out dir"));
    std::fs::create_dir_all(&out).unwrap();
    let jobs: Vec<Job> = serde_json::from_str(&std::fs::read_to_string(&list).unwrap()).unwrap();
    let root = std::env::current_dir().unwrap();
    let config = AiConfig::load(&root).expect("config");
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &process_environment,
    };
    let mut keys = HashMap::new();
    if let Ok(Some((key, _))) = secrets.resolve(Provider::Huniu) {
        keys.insert(Provider::Huniu, key);
    }
    let work = root.join("target/gen-images");
    let settings = ServiceSettings {
        language: None,
        huniu_base_url: config.huniu.base_url.clone(),
        volc_base_url: config.volc.base_url.clone(),
        huniu_strategy: config.huniu.strategy.clone(),
        keys,
        bindings: HashMap::from([(Capability::Image, Provider::Huniu)]),
        models: HashMap::new(),
        journal_dir: work.join("tasks"),
        artifacts_root: work.join("blobs"),
        proxy: config.proxy,
        limits: config.limits.clone(),
        retry: RetryPolicy::default(),
        breaker: BreakerPolicy::default(),
    };
    let (service, _) = AiService::start(settings).expect("service");
    let mut pending = HashMap::new();
    for job in &jobs {
        let key = service.submit(TaskSpec {
            request: TaskRequest::Image(ImageRequest {
                model: config.huniu.image_model.clone(),
                prompt: job.prompt.clone(),
                size: job.size.clone(),
                quality: job.quality.clone(),
                transparent: job.transparent,
                variant: job.variant,
                references: job.references.clone(),
                reference_digests: Vec::new(),
            }),
        });
        pending.insert(key, job.id.clone());
    }
    let started = Instant::now();
    let (mut done, mut failed) = (0, 0);
    while !pending.is_empty() {
        let Some(event) = service.recv_timeout(Duration::from_secs(2)) else {
            if started.elapsed() > Duration::from_secs(3600) {
                println!("timed out with {} left", pending.len());
                break;
            }
            continue;
        };
        match event {
            AiEvent::Done { key, artifact, .. } => {
                let Some(id) = pending.remove(&key) else { continue };
                match artifact {
                    Some(artifact) => {
                        let target = out.join(format!("{id}.{}", artifact.ext));
                        std::fs::copy(service.artifact_path(&artifact), &target).unwrap();
                        done += 1;
                        println!(
                            "[{:>4.0}s] ok   {id} -> {}",
                            started.elapsed().as_secs_f64(),
                            target.display()
                        );
                    }
                    None => {
                        failed += 1;
                        println!("no image for {id}");
                    }
                }
            }
            AiEvent::Failed { key, message, .. } => {
                if let Some(id) = pending.remove(&key) {
                    failed += 1;
                    println!("[{:>4.0}s] FAIL {id}: {message}", started.elapsed().as_secs_f64());
                }
            }
            AiEvent::NeedsAttention { .. } | AiEvent::Waiting { .. } => println!("{event:?}"),
            _ => {}
        }
    }
    println!("done {done}, failed {failed}");
}
