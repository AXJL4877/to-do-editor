//! One small real request through the service, to confirm provider contracts.
//! Spends money (fractions of a cent). Results go to `target/ai-probe/`.
//! `cargo run -p todo_ai --example ai_probe -- tts|chat|chat-json|image`
use std::collections::HashMap;
use std::time::Duration;
use todo_ai::net::{BreakerPolicy, RetryPolicy};
use todo_ai::secrets::{process_environment, Secrets};
use todo_ai::{AiConfig, AiEvent, AiService, ServiceSettings};
use todo_production::*;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "tts".into());
    let root = std::env::current_dir().unwrap();
    let config = AiConfig::load(&root).expect("config");
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &process_environment,
    };
    let mut keys = HashMap::new();
    for provider in [Provider::Huniu, Provider::Volc] {
        if let Ok(Some((key, _))) = secrets.resolve(provider) {
            keys.insert(provider, key);
        }
    }
    let probe = root.join("target/ai-probe");
    let settings = ServiceSettings {
        huniu_base_url: config.huniu.base_url.clone(),
        volc_base_url: config.volc.base_url.clone(),
        huniu_strategy: config.huniu.strategy.clone(),
        keys,
        journal_dir: probe.join("tasks"),
        artifacts_root: probe.join("blobs"),
        proxy: config.proxy,
        limits: config.limits.clone(),
        retry: RetryPolicy {
            paid_retries: 0,
            ..RetryPolicy::default()
        },
        breaker: BreakerPolicy::default(),
    };
    let spec = match mode.as_str() {
        "tts" => TaskSpec {
            provider: Provider::Volc,
            request: TaskRequest::Speech(SpeechRequest {
                text: "复利靠的是时间，越早开始越轻松。".into(),
                speaker: "zh_female_mizai_uranus_bigtts".into(),
                resource_id: config.volc.resource_id.clone(),
                sample_rate: 24000,
            }),
        },
        "chat" | "chat-json" => TaskSpec {
            provider: Provider::Huniu,
            request: TaskRequest::Chat(ChatRequest {
                model: config.huniu.chat_model.clone(),
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "只回复一个 JSON 对象：{\"ok\": true, \"word\": \"<任意一个中文词>\"}".into(),
                }],
                temperature: 0.2,
                json: mode == "chat-json",
            }),
        },
        "image" => TaskSpec {
            provider: Provider::Huniu,
            request: TaskRequest::Image(ImageRequest {
                model: config.huniu.image_model.clone(),
                prompt: "ONE chalk drawing of a small house on a pure flat black background, centered, clear margin".into(),
                size: "1024x1024".into(),
                quality: Some("low".into()),
                transparent: false,
                variant: 0,
            }),
        },
        other => panic!("unknown mode {other}"),
    };
    let (service, recovered) = AiService::start(settings).expect("service");
    if !recovered.is_empty() {
        println!("recovered: {recovered:?}");
    }
    let started = std::time::Instant::now();
    let key = service.submit(spec);
    loop {
        let Some(event) = service.recv_timeout(Duration::from_secs(1)) else {
            if started.elapsed() > Duration::from_secs(400) {
                println!("timed out");
                return;
            }
            continue;
        };
        match &event {
            AiEvent::Done {
                artifact, output, ..
            } => {
                println!("done in {:.1}s", started.elapsed().as_secs_f64());
                if let Some(artifact) = artifact {
                    println!(
                        "artifact {} ({} bytes)",
                        service.artifact_path(artifact).display(),
                        artifact.bytes
                    );
                }
                if let Some(output) = output {
                    let mut shown = output.clone();
                    if let Some(words) = shown.get_mut("words").and_then(|w| w.as_array_mut()) {
                        let total = words.len();
                        words.truncate(6);
                        println!("words: {total}");
                    }
                    if let Some(sentences) = shown.get_mut("sentences") {
                        *sentences = serde_json::json!("…");
                    }
                    println!("output {shown}");
                }
                return;
            }
            AiEvent::Failed { .. } => {
                println!("{event:?}");
                return;
            }
            AiEvent::Waiting { .. } | AiEvent::NeedsAttention { .. } => println!("{event:?}"),
            _ => {}
        }
        let _ = &key;
    }
}
