//! huniuai.com (OpenAI-compatible). Chat streams over SSE; image generation is
//! a long synchronous POST whose result is fetched separately, so a generated
//! image is downloaded again instead of generated again (`download_failed`
//! keeps its URL in the 密匙 toolkit for the same reason).
use super::{AttemptError, Outcome};
use crate::artifacts::{verify_image, ArtifactStore, Part};
use crate::net::http::{next_chunk, read_body, Http, Request};
use crate::net::ErrorClass;
use crate::Secret;
use base64::Engine;
use std::io::Write;
use std::time::Duration;
use todo_production::{ChatRequest, ChatRole, ImageRequest};

/// The toolkit's `MISHI_IMAGE_TIMEOUT` default.
pub const IMAGE_HEAD_TIMEOUT: Duration = Duration::from_secs(300);

fn auth(key: &Secret) -> (&'static str, String) {
    ("Authorization", format!("Bearer {}", key.expose()))
}

pub fn chat_body(request: &ChatRequest) -> serde_json::Value {
    let messages: Vec<_> = request
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                ChatRole::System => "system",
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
            };
            serde_json::json!({ "role": role, "content": message.content })
        })
        .collect();
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": messages,
        "temperature": request.temperature,
        "stream": true,
    });
    if request.json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    body
}

/// Accumulates `choices[0].delta.content` from SSE, or a plain JSON answer
/// from gateways that ignore `stream`.
#[derive(Debug, Default)]
pub struct ChatParser {
    pending: Vec<u8>,
    raw: Vec<u8>,
    pub text: String,
    pub done: bool,
    saw_event: bool,
}

impl ChatParser {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.raw.extend_from_slice(bytes);
        self.pending.extend_from_slice(bytes);
        while let Some(position) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=position).collect();
            self.line(&String::from_utf8_lossy(&line));
        }
    }

    pub fn finish(&mut self) -> Result<String, AttemptError> {
        let rest = std::mem::take(&mut self.pending);
        self.line(&String::from_utf8_lossy(&rest));
        if !self.saw_event {
            // Not SSE: a complete chat completion object.
            let value: serde_json::Value = serde_json::from_slice(&self.raw).map_err(|e| {
                AttemptError::new(ErrorClass::Transient, format!("chat reply: {e}"))
            })?;
            self.text = value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
        } else if !self.done {
            return Err(AttemptError::new(
                ErrorClass::Transient,
                "chat stream ended early",
            ));
        }
        if self.text.trim().is_empty() {
            return Err(AttemptError::new(
                ErrorClass::Transient,
                "chat returned no text",
            ));
        }
        Ok(std::mem::take(&mut self.text))
    }

    fn line(&mut self, line: &str) {
        let Some(data) = line.trim().strip_prefix("data:") else {
            return;
        };
        self.saw_event = true;
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(piece) = value["choices"][0]["delta"]["content"].as_str() {
                self.text.push_str(piece);
            }
            if value["choices"][0]["finish_reason"].is_string() {
                self.done = true;
            }
        }
    }
}

pub async fn chat(
    http: &Http,
    base_url: &str,
    key: &Secret,
    request: &ChatRequest,
) -> Result<Outcome, AttemptError> {
    let mut response = http
        .send(Request {
            method: reqwest::Method::POST,
            url: format!("{base_url}/v1/chat/completions"),
            headers: vec![auth(key), ("Content-Type", "application/json".into())],
            body: Some(serde_json::to_vec(&chat_body(request)).expect("json")),
            head_timeout: Duration::from_secs(120),
            paid: false,
        })
        .await
        .map_err(AttemptError::from)?;
    let mut parser = ChatParser::default();
    // Streaming tokens keep arriving; silence means a stalled connection.
    while let Some(chunk) = next_chunk(&mut response, Duration::from_secs(45), false)
        .await
        .map_err(AttemptError::from)?
    {
        parser.feed(&chunk);
    }
    let text = parser.finish()?;
    Ok(Outcome {
        artifact: None,
        output: Some(serde_json::json!({ "text": text })),
    })
}

/// Discover models from an OpenAI-compatible gateway. This is a free GET and
/// never enters the paid task journal.
pub async fn list_models(
    http: &Http,
    base_url: &str,
    key: &Secret,
) -> Result<Vec<crate::models::ModelInfo>, AttemptError> {
    let mut response = http
        .send(Request {
            method: reqwest::Method::GET,
            url: format!("{base_url}/v1/models"),
            headers: vec![auth(key)],
            body: None,
            head_timeout: Duration::from_secs(20),
            paid: false,
        })
        .await
        .map_err(AttemptError::from)?;
    let body = read_body(
        &mut response,
        Duration::from_secs(10),
        4 * 1024 * 1024,
        false,
    )
    .await
    .map_err(AttemptError::from)?;
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AttemptError::new(ErrorClass::Invalid, format!("models response: {e}")))?;
    let models = crate::models::classify_models(&value);
    if models.is_empty() {
        return Err(AttemptError::new(
            ErrorClass::Invalid,
            "models response contained no usable model ids",
        ));
    }
    Ok(models)
}

pub fn image_body(request: &ImageRequest, strategy: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": request.model,
        "prompt": request.prompt,
        "n": 1,
        "size": request.size,
    });
    if let Some(quality) = &request.quality {
        body["quality"] = quality.clone().into();
    }
    if request.transparent {
        body["background"] = "transparent".into();
        body["output_format"] = "png".into();
    }
    if let Some(strategy) = strategy {
        body["strategy"] = strategy.into();
    }
    body
}

/// Where a generated image is.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageResult {
    Url(String),
    Inline(Vec<u8>),
}

pub fn parse_image_result(body: &[u8]) -> Result<ImageResult, AttemptError> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| AttemptError::new(ErrorClass::Ambiguous, format!("image reply: {e}")))?;
    let item = &value["data"][0];
    if let Some(url) = item["url"].as_str().or_else(|| item["image_url"].as_str()) {
        return Ok(ImageResult::Url(url.to_owned()));
    }
    if let Some(b64) = item["b64_json"].as_str() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| AttemptError::new(ErrorClass::Ambiguous, format!("image base64: {e}")))?;
        return Ok(ImageResult::Inline(bytes));
    }
    Err(AttemptError::new(
        ErrorClass::Ambiguous,
        format!(
            "image reply without data: {}",
            String::from_utf8_lossy(body)
                .chars()
                .take(300)
                .collect::<String>()
        ),
    ))
}

/// The billed step. Its failures after sending are ambiguous.
pub async fn submit_image(
    http: &Http,
    base_url: &str,
    key: &Secret,
    request: &ImageRequest,
    strategy: Option<&str>,
) -> Result<ImageResult, AttemptError> {
    let mut response = http
        .send(Request {
            method: reqwest::Method::POST,
            url: format!("{base_url}/v1/images/generations"),
            headers: vec![auth(key), ("Content-Type", "application/json".into())],
            body: Some(serde_json::to_vec(&image_body(request, strategy)).expect("json")),
            head_timeout: IMAGE_HEAD_TIMEOUT,
            paid: true,
        })
        .await
        .map_err(AttemptError::from)?;
    let body = read_body(
        &mut response,
        Duration::from_secs(120),
        256 * 1024 * 1024,
        true,
    )
    .await
    .map_err(AttemptError::from)?;
    parse_image_result(&body)
}

fn same_host(base_url: &str, url: &str) -> bool {
    let host = |u: &str| {
        u.split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .map(str::to_ascii_lowercase)
    };
    host(base_url).is_some() && host(base_url) == host(url)
}

/// Free to repeat: resumes with `Range` and verifies before publishing.
pub async fn download_image(
    http: &Http,
    base_url: &str,
    key: &Secret,
    url: &str,
    store: &ArtifactStore,
    owner: &str,
) -> Result<Outcome, AttemptError> {
    let part = store.part(owner).map_err(AttemptError::storage)?;
    let mut headers = Vec::new();
    if same_host(base_url, url) {
        headers.push(auth(key));
    }
    let resume = part.len();
    if resume > 0 {
        headers.push(("Range", format!("bytes={resume}-")));
    }
    let mut response = http
        .send(Request {
            method: reqwest::Method::GET,
            url: url.to_owned(),
            headers,
            body: None,
            head_timeout: Duration::from_secs(60),
            paid: false,
        })
        .await
        .map_err(AttemptError::from)?;
    if resume > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        part.reset().map_err(AttemptError::storage)?;
    }
    let mut file = part.append().map_err(AttemptError::storage)?;
    while let Some(chunk) = next_chunk(&mut response, Duration::from_secs(60), false)
        .await
        .map_err(AttemptError::from)?
    {
        file.write_all(&chunk)
            .map_err(|e| AttemptError::new(ErrorClass::Transient, e.to_string()))?;
    }
    drop(file);
    publish_image(store, part)
}

pub fn store_inline_image(
    store: &ArtifactStore,
    owner: &str,
    bytes: &[u8],
) -> Result<Outcome, AttemptError> {
    let part = store.part(owner).map_err(AttemptError::storage)?;
    part.reset().map_err(AttemptError::storage)?;
    part.append()
        .map_err(AttemptError::storage)?
        .write_all(bytes)
        .map_err(|e| AttemptError::new(ErrorClass::Transient, e.to_string()))?;
    publish_image(store, part)
}

fn publish_image(store: &ArtifactStore, part: Part) -> Result<Outcome, AttemptError> {
    let mut size = (0, 0);
    let ext = image::ImageReader::open(part.path())
        .ok()
        .and_then(|reader| reader.with_guessed_format().ok())
        .and_then(|reader| reader.format())
        .map_or("png", |format| match format {
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => "png",
        });
    // A truncated file fails here and is discarded; the URL stays for a re-download.
    let artifact = store
        .publish(part, ext, |path| {
            verify_image(path, (64, 64)).map(|s| size = s)
        })
        .map_err(|error| match error {
            crate::ArtifactError::Invalid(reason) => {
                AttemptError::new(ErrorClass::Transient, reason)
            }
            other => AttemptError::storage(other),
        })?;
    Ok(Outcome {
        artifact: Some(artifact),
        output: Some(serde_json::json!({ "width": size.0, "height": size.1 })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use todo_production::ChatMessage;

    #[test]
    fn chat_streams_and_plain_replies() {
        let mut parser = ChatParser::default();
        for chunk in [
            "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},",
            "\"finish_reason\":null}]}\n\ndata: [DONE]\n",
        ] {
            parser.feed(chunk.as_bytes());
        }
        assert_eq!(parser.finish().unwrap(), "你好");

        let mut plain = ChatParser::default();
        plain.feed(br#"{"choices":[{"message":{"content":"{\"a\":1}"}}]}"#);
        assert_eq!(plain.finish().unwrap(), r#"{"a":1}"#);

        let mut cut = ChatParser::default();
        cut.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n");
        assert_eq!(cut.finish().unwrap_err().class, ErrorClass::Transient);
    }

    #[test]
    fn bodies() {
        let chat = chat_body(&ChatRequest {
            model: "gpt-6-astra".into(),
            messages: vec![ChatMessage {
                role: ChatRole::System,
                content: "s".into(),
            }],
            temperature: 0.3,
            json: true,
        });
        assert_eq!(chat["response_format"]["type"], "json_object");
        assert_eq!(chat["messages"][0]["role"], "system");
        let image = image_body(
            &ImageRequest {
                model: "gpt-image-2".into(),
                prompt: "p".into(),
                size: "1024x1024".into(),
                quality: None,
                transparent: true,
                variant: 3,
            },
            Some("price_first"),
        );
        assert_eq!(image["background"], "transparent");
        assert_eq!(image["output_format"], "png");
        assert_eq!(image["strategy"], "price_first");
        assert!(
            image.get("variant").is_none(),
            "variants only distinguish tasks locally"
        );
    }

    #[test]
    fn image_results() {
        assert_eq!(
            parse_image_result(br#"{"data":[{"url":"https://cdn/x.png"}]}"#).unwrap(),
            ImageResult::Url("https://cdn/x.png".into())
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"img");
        assert_eq!(
            parse_image_result(format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#).as_bytes())
                .unwrap(),
            ImageResult::Inline(b"img".to_vec())
        );
        assert_eq!(
            parse_image_result(b"{}").unwrap_err().class,
            ErrorClass::Ambiguous
        );
        assert!(same_host(
            "https://huniuai.com",
            "https://HUNIUAI.com/files/a.png"
        ));
        assert!(!same_host(
            "https://huniuai.com",
            "https://assets.token6688.com/a.png"
        ));
    }
}
