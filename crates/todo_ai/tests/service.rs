//! The executor against a fault-injecting local server: no network, no cost.
mod support;

use base64::Engine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use support::{Behaviour, FakeServer};
use todo_ai::config::Limits;
use todo_ai::net::{BreakerPolicy, RetryPolicy};
use todo_ai::{
    AiEvent, AiService, Attention, Journal, ProxyMode, Secret, ServiceSettings, TaskState,
    WaitReason,
};
use todo_production::*;

fn scratch(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ai-service-tests")
        .join(format!("{name}-{unique}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn settings(server: &FakeServer, dir: &Path) -> ServiceSettings {
    ServiceSettings {
        huniu_base_url: server.base.clone(),
        volc_base_url: server.base.clone(),
        huniu_strategy: None,
        keys: HashMap::from([
            (Provider::Huniu, Secret::new("sk-good").unwrap()),
            (Provider::Volc, Secret::new("volc-key").unwrap()),
        ]),
        journal_dir: dir.join("tasks"),
        artifacts_root: dir.join("blobs"),
        proxy: ProxyMode::Direct,
        limits: Limits {
            chat_inflight: 4,
            image_inflight: 4,
            image_rate_per_second: 50.0,
            speech_inflight: 2,
            image_paid_retries: 2,
        },
        retry: RetryPolicy {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(40),
            max_retry_after: Duration::from_millis(200),
            paid_retries: 2,
        },
        breaker: BreakerPolicy {
            threshold: 2,
            window: Duration::from_secs(10),
            open_for: Duration::from_millis(150),
        },
    }
}

fn chat_spec(prompt: &str) -> TaskSpec {
    TaskSpec {
        provider: Provider::Huniu,
        request: TaskRequest::Chat(ChatRequest {
            model: "gpt-6-astra".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: prompt.into(),
            }],
            temperature: 0.3,
            json: false,
        }),
    }
}

fn image_spec(prompt: &str) -> TaskSpec {
    TaskSpec {
        provider: Provider::Huniu,
        request: TaskRequest::Image(ImageRequest {
            model: "gpt-image-2".into(),
            prompt: prompt.into(),
            size: "1024x1024".into(),
            quality: None,
            transparent: false,
            variant: 0,
        }),
    }
}

fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(128, 96, image::Rgba([200, 30, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

const SSE_OK: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"好的\"}}]}\n\ndata: [DONE]\n\n";

/// Events until one for `key` is final; panics after `limit`.
fn until_final(service: &AiService, key: &TaskKey, limit: Duration) -> (AiEvent, Vec<AiEvent>) {
    let deadline = std::time::Instant::now() + limit;
    let mut seen = Vec::new();
    while std::time::Instant::now() < deadline {
        if let Some(event) = service.recv_timeout(Duration::from_millis(50)) {
            let finished = match &event {
                AiEvent::Done { key: k, .. } | AiEvent::Failed { key: k, .. } => k == key,
                _ => false,
            };
            if finished {
                return (event, seen);
            }
            seen.push(event);
        }
    }
    panic!("no final event for {key}; saw {seen:#?}");
}

#[test]
fn model_discovery_lists_and_classifies_gateway_models() {
    let server = FakeServer::start();
    server.on(
        "/v1/models",
        vec![Behaviour::json(
            200,
            r#"{"data":[{"id":"geminiflash3.1"},{"id":"seedream-4"},{"id":"sora-video"}]}"#,
        )],
    );
    let dir = scratch("model-discovery");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let id = service.list_models(Provider::Huniu);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let event = service.recv_timeout(Duration::from_millis(50));
        if let Some(AiEvent::Models {
            id: event_id,
            models,
            ..
        }) = event
        {
            assert_eq!(event_id, id);
            assert!(models.iter().any(|model| {
                model.id == "geminiflash3.1" && model.supports(todo_ai::ModelCapability::Text)
            }));
            assert!(models.iter().any(|model| {
                model.id == "seedream-4" && model.supports(todo_ai::ModelCapability::Image)
            }));
            assert!(models.iter().any(|model| {
                model.id == "sora-video" && model.supports(todo_ai::ModelCapability::Video)
            }));
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "model discovery timed out"
        );
    }
}

#[test]
fn finished_requests_are_answered_from_the_journal() {
    let server = FakeServer::start();
    server.on("/v1/chat/completions", vec![Behaviour::text(200, SSE_OK)]);
    let dir = scratch("cache");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(chat_spec("hi"));
    let (done, _) = until_final(&service, &key, Duration::from_secs(10));
    assert!(
        matches!(done, AiEvent::Done { ref output, .. } if output.as_ref().unwrap()["text"] == "好的")
    );
    drop(service);
    // A new session (restart) answers from disk without another request.
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(chat_spec("hi"));
    until_final(&service, &key, Duration::from_secs(5));
    assert_eq!(server.count("POST", "/v1/chat/completions"), 1);
    let sent = &server.recorded("/v1/chat/completions")[0];
    assert_eq!(sent.headers["authorization"], "Bearer sk-good");
}

#[test]
fn an_image_is_generated_once_and_its_download_resumes() {
    let server = FakeServer::start();
    let image = png();
    let url = format!("{}/files/a.png", server.base);
    server.on(
        "/v1/images/generations",
        vec![Behaviour::json(
            200,
            &format!(r#"{{"data":[{{"url":"{url}"}}]}}"#),
        )],
    );
    server.on(
        "/files/a.png",
        vec![
            Behaviour::Truncate {
                status: 200,
                body: image.clone(),
                cut: image.len() / 2,
            },
            Behaviour::File {
                body: image.clone(),
            },
        ],
    );
    let dir = scratch("image");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(image_spec("chalk house"));
    let (done, _) = until_final(&service, &key, Duration::from_secs(10));
    let AiEvent::Done {
        artifact: Some(artifact),
        output,
        ..
    } = done
    else {
        panic!("{done:?}");
    };
    assert_eq!(
        std::fs::read(service.artifact_path(&artifact)).unwrap(),
        image
    );
    assert_eq!(output.unwrap()["width"], 128);
    assert_eq!(
        server.count("POST", "/v1/images/generations"),
        1,
        "never billed twice"
    );
    let downloads = server.recorded("/files/a.png");
    assert_eq!(downloads.len(), 2);
    assert!(
        !downloads[1].headers["range"].is_empty(),
        "second attempt resumes"
    );
    // Same-origin result URLs carry the key, third-party CDNs would not.
    assert_eq!(downloads[0].headers["authorization"], "Bearer sk-good");
}

#[test]
fn unknown_billed_outcomes_stop_after_the_budget() {
    let server = FakeServer::start();
    server.on("/v1/images/generations", vec![Behaviour::Drop]);
    let dir = scratch("ambiguous");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(image_spec("dropped"));
    let (failed, _) = until_final(&service, &key, Duration::from_secs(10));
    assert!(
        matches!(
            failed,
            AiEvent::Failed {
                possibly_billed: true,
                ..
            }
        ),
        "{failed:?}"
    );
    assert_eq!(
        server.count("POST", "/v1/images/generations"),
        3,
        "first try + 2 paid retries"
    );
    drop(service);
    let (journal, _) = Journal::open(dir.join("tasks"), 2, 0).unwrap();
    let record = journal.get(&key).unwrap();
    assert_eq!(
        (record.state, record.submits, record.paid_retries),
        (TaskState::Ambiguous, 3, 2)
    );
}

#[test]
fn an_outage_opens_the_circuit_and_recovers_without_help() {
    let server = FakeServer::start();
    server.on(
        "/v1/chat/completions",
        vec![
            Behaviour::text(502, "<html>bad gateway</html>"),
            Behaviour::text(503, "busy"),
            Behaviour::text(502, "<html>bad gateway</html>"),
            Behaviour::text(200, SSE_OK),
        ],
    );
    let dir = scratch("outage");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(chat_spec("outage"));
    let (done, seen) = until_final(&service, &key, Duration::from_secs(10));
    assert!(matches!(done, AiEvent::Done { .. }), "{done:?}");
    assert!(seen.iter().any(|e| matches!(
        e,
        AiEvent::Waiting {
            reason: WaitReason::Network,
            ..
        }
    )));
    assert_eq!(server.count("POST", "/v1/chat/completions"), 4);
}

#[test]
fn a_rejected_key_pauses_until_it_is_replaced() {
    let server = FakeServer::start();
    server.on(
        "/v1/chat/completions",
        vec![Behaviour::Custom(Arc::new(|request| {
            if request.headers["authorization"] == "Bearer sk-new" {
                (200, SSE_OK.as_bytes().to_vec())
            } else {
                (401, br#"{"error":{"message":"invalid api key"}}"#.to_vec())
            }
        }))],
    );
    let dir = scratch("auth");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(chat_spec("auth"));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline, "no attention event");
        if let Some(AiEvent::NeedsAttention {
            provider,
            attention,
        }) = service.recv_timeout(Duration::from_millis(50))
        {
            assert_eq!((provider, attention), (Provider::Huniu, Attention::Auth));
            break;
        }
    }
    service.set_key(Provider::Huniu, Secret::new("sk-new"));
    let (done, _) = until_final(&service, &key, Duration::from_secs(10));
    assert!(matches!(done, AiEvent::Done { .. }), "{done:?}");
}

#[test]
fn a_restart_resumes_downloading_without_generating_again() {
    let server = FakeServer::start();
    let image = png();
    let url = format!("{}/files/b.png", server.base);
    server.on(
        "/files/b.png",
        vec![Behaviour::File {
            body: image.clone(),
        }],
    );
    server.on(
        "/v1/images/generations",
        vec![Behaviour::text(500, "must not be called")],
    );
    let dir = scratch("restart");
    let spec = image_spec("already generated");
    {
        let (mut journal, _) = Journal::open(dir.join("tasks"), 2, 0).unwrap();
        journal.ensure(&spec, 0).unwrap();
        journal
            .update(&spec.key(), 0, |r| {
                r.state = TaskState::Downloading;
                r.submits = 1;
                r.remote_url = Some(url.clone());
            })
            .unwrap();
    }
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(spec);
    let (done, _) = until_final(&service, &key, Duration::from_secs(10));
    assert!(matches!(done, AiEvent::Done { .. }), "{done:?}");
    assert_eq!(server.count("POST", "/v1/images/generations"), 0);
}

#[test]
fn speech_streams_audio_and_word_times_into_a_wav() {
    let server = FakeServer::start();
    let audio = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 4800]);
    let body = format!(
        "{{\"code\":0,\"data\":\"{audio}\"}}\n\
         {{\"code\":0,\"sentence\":{{\"text\":\"你好\",\"words\":[{{\"word\":\"你\",\"startTime\":0.0,\"endTime\":0.05}},{{\"word\":\"好\",\"startTime\":0.05,\"endTime\":0.1}}]}}}}\n\
         {{\"code\":20000000,\"message\":\"ok\"}}\n"
    );
    server.on(
        "/api/v3/tts/unidirectional",
        vec![Behaviour::text(200, &body)],
    );
    let dir = scratch("speech");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(TaskSpec {
        provider: Provider::Volc,
        request: TaskRequest::Speech(SpeechRequest {
            text: "你好".into(),
            speaker: "zh_female_mizai_uranus_bigtts".into(),
            resource_id: "seed-tts-2.0".into(),
            sample_rate: 24000,
        }),
    });
    let (done, _) = until_final(&service, &key, Duration::from_secs(10));
    let AiEvent::Done {
        artifact: Some(artifact),
        output: Some(output),
        ..
    } = done
    else {
        panic!("{done:?}");
    };
    let wav = std::fs::read(service.artifact_path(&artifact)).unwrap();
    assert_eq!(wav.len(), 44 + 4800);
    assert_eq!(output["words"][1]["word"], "好");
    let sent = &server.recorded("/api/v3/tts/unidirectional")[0];
    assert_eq!(sent.headers["x-api-key"], "volc-key");
    assert_eq!(sent.headers["x-api-resource-id"], "seed-tts-2.0");
}

#[test]
fn invalid_requests_fail_at_once() {
    let server = FakeServer::start();
    server.on(
        "/v1/chat/completions",
        vec![Behaviour::json(
            400,
            r#"{"error":{"message":"unknown model"}}"#,
        )],
    );
    let dir = scratch("invalid");
    let (service, _) = AiService::start(settings(&server, &dir)).unwrap();
    let key = service.submit(chat_spec("bad"));
    let (failed, _) = until_final(&service, &key, Duration::from_secs(5));
    assert!(
        matches!(failed, AiEvent::Failed { ref class, possibly_billed: false, .. } if class == "invalid"),
        "{failed:?}"
    );
    assert_eq!(server.count("POST", "/v1/chat/completions"), 1);
}
