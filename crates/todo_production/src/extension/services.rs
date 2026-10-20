//! Pure conversions between worker service calls and host task/results.
//!
//! The app service bridge owns I/O; this module owns the request defaults and
//! the single narration/subtitle timing path shared by all callers.
use super::protocol::{ChatCall, ImageCall, ServiceCall, SpeechCall, SubtitlesCall};
use super::HostDefaults;
use crate::subtitles::timeline_cues;
use crate::tasks::{AudioRequest, ImageRequest, LanguageRequest, TaskRequest};
use crate::voice::{speech_timeline, Pcm, Timeline, Word};
use serde_json::{json, Value};

pub fn speech_resource_id(speaker: &str, default: &str) -> String {
    if speaker.starts_with("S_") {
        "seed-icl-2.0".into()
    } else {
        default.into()
    }
}

pub fn task_for(call: &ServiceCall, host: &HostDefaults) -> Option<TaskRequest> {
    match call {
        ServiceCall::Chat(ChatCall {
            messages,
            json,
            temperature,
            model,
        }) => Some(TaskRequest::Language(LanguageRequest {
            messages: messages.clone(),
            json: *json,
            temperature: temperature.unwrap_or(0.3),
            model: model.clone().unwrap_or_default(),
        })),
        ServiceCall::Image(ImageCall {
            prompt,
            size,
            quality,
            transparent,
            variant,
            model,
            references,
        }) => Some(TaskRequest::Image(ImageRequest {
            prompt: prompt.clone(),
            size: size.clone(),
            quality: quality.clone(),
            transparent: *transparent,
            variant: *variant,
            // Still workdir-relative here; the host resolves them.
            references: references.clone(),
            reference_digests: Vec::new(),
            model: model.clone().unwrap_or_default(),
        })),
        ServiceCall::Speech(SpeechCall {
            text,
            voice,
            sample_rate,
            rate,
        }) => Some(TaskRequest::Audio(AudioRequest {
            model: String::new(),
            text: text.clone(),
            speaker: voice.clone(),
            resource_id: speech_resource_id(voice, &host.audio_model),
            sample_rate: sample_rate.unwrap_or(host.sample_rate),
            speech_rate: *rate,
        })),
        ServiceCall::Subtitles(_) | ServiceCall::Probe(_) | ServiceCall::Encode(_) => None,
    }
}

pub fn speech_result(text: &str, raw: &Value, pcm: Pcm<'_>, file: &str) -> Value {
    let sentences = raw
        .get("sentences")
        .or_else(|| raw.get("sentencesRaw"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let words: Vec<Word> = raw
        .get("words")
        .and_then(Value::as_array)
        .cloned()
        .map(Value::Array)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let timeline = speech_timeline(text, &sentences, &words, pcm);
    json!({ "file": file, "duration": timeline.duration, "timeline": timeline })
}

pub fn subtitles_result(call: &SubtitlesCall, timeline: &Timeline, pcm: Pcm<'_>) -> Value {
    let cues = timeline_cues(&call.script, timeline, pcm, call.max_chars.unwrap_or(18));
    json!({ "cues": cues })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::protocol::{ChatCall, SpeechCall};
    use crate::tasks::ChatRole;
    use serde_json::json;
    use std::path::Path;

    fn host() -> HostDefaults {
        HostDefaults {
            audio_model: "seed-tts-2.0".into(),
            sample_rate: 24000,
        }
    }

    #[test]
    fn task_defaults_and_clone_resource_are_host_owned() {
        let chat = task_for(
            &ServiceCall::Chat(ChatCall {
                messages: vec![crate::tasks::ChatMessage {
                    role: ChatRole::User,
                    content: "hi".into(),
                }],
                json: true,
                temperature: None,
                model: None,
            }),
            &host(),
        )
        .unwrap();
        assert!(matches!(chat, TaskRequest::Language(_)));
        let speech = task_for(
            &ServiceCall::Speech(SpeechCall {
                text: "你好".into(),
                voice: "S_clone".into(),
                sample_rate: None,
                rate: Some(16),
            }),
            &host(),
        )
        .unwrap();
        match speech {
            TaskRequest::Audio(request) => assert_eq!(request.resource_id, "seed-icl-2.0"),
            _ => panic!("speech request"),
        }
    }

    #[test]
    fn speech_result_builds_one_shared_timeline() {
        let raw = json!({"words":[{"word":"你","start":0.0,"end":0.2},{"word":"好","start":0.2,"end":0.4}],"sentences":[{"text":"你好","words":[{"word":"你","startTime":0.0,"endTime":0.2},{"word":"好","startTime":0.2,"endTime":0.4}]}]});
        let samples = vec![1000i16; 9600];
        let output = speech_result(
            "你好",
            &raw,
            Pcm {
                samples: &samples,
                rate: 24000,
            },
            "services/speech.wav",
        );
        assert_eq!(output["file"], "services/speech.wav");
        assert!(output["timeline"]["chars"].as_array().unwrap().len() >= 2);
        assert!(output["duration"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn local_subtitle_calls_do_not_become_paid_tasks() {
        let call = ServiceCall::Subtitles(SubtitlesCall {
            script: "你好".into(),
            speech: "services/speech.wav".into(),
            max_chars: Some(18),
        });
        assert!(task_for(&call, &host()).is_none());
        assert!(Path::new("services/speech.wav").is_relative());
    }
}
