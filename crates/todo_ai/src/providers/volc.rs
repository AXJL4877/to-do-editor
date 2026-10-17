//! Volcengine (Doubao) Seed TTS v3, unidirectional HTTP stream with word
//! timestamps. Same request as the 密匙 toolkit (`media/tts.py`), except the
//! audio is requested as raw PCM and wrapped in a WAV header here: streamed
//! WAV headers declare 0xFFFFFFFF sizes that break downstream readers.
use super::{AttemptError, Outcome};
use crate::artifacts::{verify_wav, ArtifactStore};
use crate::net::http::{next_chunk, Http, Request};
use crate::net::ErrorClass;
use crate::Secret;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::time::Duration;
use todo_production::SpeechRequest;

const PATH: &str = "/api/v3/tts/unidirectional";
const END_CODE: i64 = 20_000_000;

/// A word with its time in seconds from the start of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

/// Request body; `speaker` ids starting with `S_` are cloned voices.
pub fn request_body(request: &SpeechRequest) -> serde_json::Value {
    let additions = serde_json::json!({ "disable_markdown_filter": true }).to_string();
    serde_json::json!({
        "user": { "uid": "todo-editor" },
        "req_params": {
            "text": request.text,
            "speaker": request.speaker,
            "audio_params": {
                "format": "pcm",
                "sample_rate": request.sample_rate,
                "enable_subtitle": true,
            },
            // The API expects this one as a JSON string.
            "additions": additions,
        }
    })
}

/// Incremental parser for the response: NDJSON or SSE `data:` lines, possibly
/// several objects on one line.
#[derive(Debug, Default)]
pub struct StreamParser {
    pending: Vec<u8>,
    pub pcm: Vec<u8>,
    pub words: Vec<Word>,
    pub sentences: Vec<serde_json::Value>,
    pub finished: bool,
}

impl StreamParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), AttemptError> {
        self.pending.extend_from_slice(bytes);
        while let Some(position) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=position).collect();
            self.line(&String::from_utf8_lossy(&line))?;
        }
        Ok(())
    }

    /// The stream ended: parse what is left without a trailing newline.
    pub fn finish(&mut self) -> Result<(), AttemptError> {
        let rest = std::mem::take(&mut self.pending);
        self.line(&String::from_utf8_lossy(&rest))
    }

    fn line(&mut self, line: &str) -> Result<(), AttemptError> {
        let line = line.trim();
        if line.is_empty() || line.starts_with("event:") || line.starts_with(':') {
            return Ok(());
        }
        let json = line.strip_prefix("data:").map_or(line, str::trim);
        for value in serde_json::Deserializer::from_str(json).into_iter::<serde_json::Value>() {
            let Ok(value) = value else {
                // Not JSON (keep-alive noise); the toolkit skips these too.
                return Ok(());
            };
            self.object(&value)?;
        }
        Ok(())
    }

    fn object(&mut self, value: &serde_json::Value) -> Result<(), AttemptError> {
        let code = value.get("code").and_then(serde_json::Value::as_i64);
        match code {
            None | Some(0) => {}
            Some(END_CODE) => {
                self.finished = true;
                return Ok(());
            }
            Some(code) => {
                let message = value
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                return Err(AttemptError::new(
                    error_class(code, message),
                    format!("tts code {code}: {message}"),
                ));
            }
        }
        if let Some(data) = value.get("data").and_then(serde_json::Value::as_str) {
            let audio = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| {
                    AttemptError::new(ErrorClass::Transient, format!("tts audio chunk: {e}"))
                })?;
            self.pcm.extend_from_slice(&audio);
        }
        if let Some(sentence) = value.get("sentence").filter(|s| s.is_object()) {
            if let Some(words) = sentence.get("words").and_then(serde_json::Value::as_array) {
                for word in words {
                    let text = word
                        .get("word")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    let time = |key| {
                        word.get(key)
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0)
                    };
                    self.words.push(Word {
                        word: text.into(),
                        start: time("startTime"),
                        end: time("endTime"),
                    });
                }
            }
            self.sentences.push(sentence.clone());
        }
        Ok(())
    }
}

/// Volcengine status codes: 45xxxxx are request problems, quota and auth
/// errors are reported in the message.
fn error_class(code: i64, message: &str) -> ErrorClass {
    let lower = message.to_lowercase();
    if lower.contains("quota")
        || lower.contains("balance")
        || lower.contains("余额")
        || lower.contains("insufficient")
    {
        ErrorClass::Billing
    } else if lower.contains("auth") || lower.contains("api key") || lower.contains("permission") {
        ErrorClass::Auth
    } else if (40_000_000..50_000_000).contains(&code) {
        ErrorClass::Invalid
    } else {
        ErrorClass::Transient
    }
}

/// 16-bit mono PCM → RIFF/WAVE.
pub fn wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

pub async fn synthesize(
    http: &Http,
    base_url: &str,
    key: &Secret,
    request: &SpeechRequest,
    store: &ArtifactStore,
    owner: &str,
) -> Result<Outcome, AttemptError> {
    let body = serde_json::to_vec(&request_body(request)).expect("json");
    let mut response = http
        .send(Request {
            method: reqwest::Method::POST,
            url: format!("{base_url}{PATH}"),
            headers: vec![
                ("Content-Type", "application/json".into()),
                ("X-Api-Key", key.expose().into()),
                ("X-Api-Resource-Id", request.resource_id.clone()),
            ],
            body: Some(body),
            head_timeout: Duration::from_secs(60),
            paid: false,
        })
        .await
        .map_err(AttemptError::from)?;
    let mut parser = StreamParser::default();
    while let Some(chunk) = next_chunk(&mut response, Duration::from_secs(60), false)
        .await
        .map_err(AttemptError::from)?
    {
        parser.feed(&chunk)?;
        if parser.finished {
            break;
        }
    }
    parser.finish()?;
    if parser.pcm.is_empty() {
        return Err(AttemptError::new(
            ErrorClass::Transient,
            "tts returned no audio",
        ));
    }
    if parser.words.is_empty() {
        return Err(AttemptError::new(
            ErrorClass::Invalid,
            "tts returned no word timestamps (enable_subtitle needs seed-tts-2.0 / seed-icl-2.0 and zh/en text)",
        ));
    }
    let bytes = wav(&parser.pcm, request.sample_rate);
    let part = store.part(owner).map_err(AttemptError::storage)?;
    part.reset().map_err(AttemptError::storage)?;
    part.append()
        .map_err(AttemptError::storage)?
        .write_all(&bytes)
        .map_err(|e| AttemptError::new(ErrorClass::Transient, e.to_string()))?;
    let artifact = store
        .publish(part, "wav", verify_wav)
        .map_err(AttemptError::storage)?;
    Ok(Outcome {
        artifact: Some(artifact),
        output: Some(serde_json::json!({
            "words": parser.words,
            "sentences": parser.sentences,
            "sampleRate": request.sample_rate,
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_matches_the_toolkit_contract() {
        let body = request_body(&SpeechRequest {
            text: "你好".into(),
            speaker: "zh_female_mizai_uranus_bigtts".into(),
            resource_id: "seed-tts-2.0".into(),
            sample_rate: 24000,
        });
        assert_eq!(body["req_params"]["audio_params"]["enable_subtitle"], true);
        assert_eq!(body["req_params"]["audio_params"]["format"], "pcm");
        assert_eq!(
            body["req_params"]["additions"],
            r#"{"disable_markdown_filter":true}"#
        );
    }

    #[test]
    fn parses_ndjson_sse_and_concatenated_objects_across_chunks() {
        let audio = base64::engine::general_purpose::STANDARD.encode([1u8, 0, 2, 0]);
        let stream = format!(
            "event: 352\ndata: {{\"code\":0,\"data\":\"{audio}\"}}\n\
             {{\"code\":0,\"sentence\":{{\"text\":\"你好\",\"words\":[{{\"word\":\"你\",\"startTime\":0.1,\"endTime\":0.3}},{{\"word\":\"好\",\"startTime\":0.3,\"endTime\":0.5}}]}}}}{{\"code\":0,\"data\":\"{audio}\"}}\n\
             {{\"code\":20000000,\"message\":\"ok\"}}"
        );
        let mut parser = StreamParser::default();
        for chunk in stream.as_bytes().chunks(7) {
            parser.feed(chunk).unwrap();
        }
        parser.finish().unwrap();
        assert!(parser.finished);
        assert_eq!(parser.pcm, [1, 0, 2, 0, 1, 0, 2, 0]);
        assert_eq!(parser.words.len(), 2);
        assert_eq!(
            parser.words[1],
            Word {
                word: "好".into(),
                start: 0.3,
                end: 0.5
            }
        );
    }

    #[test]
    fn error_codes_are_classified() {
        let mut parser = StreamParser::default();
        let error = parser
            .feed(b"{\"code\":45000001,\"message\":\"invalid speaker\"}\n")
            .unwrap_err();
        assert_eq!(error.class, ErrorClass::Invalid);
        let error = StreamParser::default()
            .feed(b"{\"code\":55000000,\"message\":\"quota exceeded\"}\n")
            .unwrap_err();
        assert_eq!(error.class, ErrorClass::Billing);
    }

    #[test]
    fn wav_header_declares_real_sizes() {
        let file = wav(&[0u8; 480], 24000);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(file[40..44].try_into().unwrap()), 480);
        assert_eq!(file.len(), 44 + 480);
    }
}
