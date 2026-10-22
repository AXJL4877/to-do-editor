//! Multimodal requests on an OpenAI-compatible gateway: the prompt plus each
//! image as a `data:` URL in one user message. Pictures are downscaled and
//! re-encoded as JPEG so a contact sheet of frames stays a small request.
use super::huniu::{auth, ChatParser};
use super::{AttemptError, Outcome};
use crate::net::http::{next_chunk, Http, Request};
use crate::net::ErrorClass;
use crate::Secret;
use base64::Engine;
use std::path::Path;
use std::time::Duration;
use todo_production::MultimodalRequest;

/// Longest edge sent to the model; review frames need layout, not pixels.
pub const MAX_EDGE: u32 = 1280;
/// More pictures than this is a request the gateway would reject anyway.
pub const MAX_MEDIA: usize = 16;

/// A picture file as a JPEG `data:` URL, scaled to fit `MAX_EDGE`.
pub fn image_data_url(path: &Path) -> Result<String, AttemptError> {
    let bytes = std::fs::read(path).map_err(|error| {
        AttemptError::new(
            ErrorClass::Invalid,
            format!("读取审核图片 {} 失败：{error}", path.display()),
        )
    })?;
    let image = image::load_from_memory(&bytes).map_err(|error| {
        AttemptError::new(
            ErrorClass::Invalid,
            format!("{} 不是可识别的图片：{error}", path.display()),
        )
    })?;
    let image = if image.width().max(image.height()) > MAX_EDGE {
        image.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&image.to_rgb8())
        .map_err(|error| AttemptError::new(ErrorClass::Invalid, error.to_string()))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

pub fn vision_body(request: &MultimodalRequest) -> Result<serde_json::Value, AttemptError> {
    if request.media.len() > MAX_MEDIA {
        return Err(AttemptError::new(
            ErrorClass::Invalid,
            format!("一次最多审核 {MAX_MEDIA} 张图片"),
        ));
    }
    let mut content = vec![serde_json::json!({ "type": "text", "text": request.prompt })];
    for media in &request.media {
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": { "url": image_data_url(Path::new(media))? }
        }));
    }
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": [{ "role": "user", "content": content }],
        "temperature": 0.2,
        "stream": true,
    });
    if request.json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    Ok(body)
}

pub async fn vision(
    http: &Http,
    base_url: &str,
    key: &Secret,
    request: &MultimodalRequest,
) -> Result<Outcome, AttemptError> {
    let body = vision_body(request)?;
    let mut response = http
        .send(Request {
            method: reqwest::Method::POST,
            url: format!("{base_url}/v1/chat/completions"),
            headers: vec![auth(key), ("Content-Type", "application/json".into())],
            body: Some(serde_json::to_vec(&body).expect("json")),
            // Uploading pictures and reading them takes longer than text.
            head_timeout: Duration::from_secs(180),
            paid: false,
        })
        .await
        .map_err(AttemptError::from)?;
    let mut parser = ChatParser::default();
    while let Some(chunk) = next_chunk(&mut response, Duration::from_secs(90), false)
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

/// Content digests of the request's pictures, so a task's fingerprint follows
/// what the model is shown and not only the file names.
pub fn media_digests(media: &[String]) -> Vec<String> {
    use sha2::{Digest, Sha256};
    media
        .iter()
        .map(|path| match std::fs::read(path) {
            Ok(bytes) => Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            Err(_) => String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_carries_scaled_jpeg_pictures() {
        // 沙箱里 %TEMP% 不可写，放到仓库 target/ 下
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/ai-vision-tests")
            .join(format!("case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("frame.png");
        image::RgbImage::from_pixel(2560, 1440, image::Rgb([10, 200, 30]))
            .save(&file)
            .unwrap();
        let request = MultimodalRequest {
            model: "vision".into(),
            prompt: "审片".into(),
            media: vec![file.to_string_lossy().into_owned()],
            json: true,
            digests: vec![],
        };
        let body = vision_body(&request).unwrap();
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "审片");
        let url = content[1]["image_url"]["url"].as_str().unwrap();
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/jpeg;base64,").unwrap())
            .unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1280, 720));
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(media_digests(&request.media)[0].len(), 64);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_pictures_are_invalid_not_transient() {
        let request = MultimodalRequest {
            model: "vision".into(),
            prompt: "审片".into(),
            media: vec!["Z:/nope/missing.png".into()],
            json: false,
            digests: vec![],
        };
        assert_eq!(vision_body(&request).unwrap_err().class, ErrorClass::Invalid);
    }
}
