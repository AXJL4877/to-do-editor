//! One HTTP client for every provider. Failures carry the stage they happened
//! in, so a billed request is only resent when it certainly never arrived.
use super::classify::{classify_status, classify_transport, parse_retry_after, ErrorClass, Stage};
use crate::config::ProxyMode;
use std::time::Duration;

pub use reqwest::Response;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Error bodies are diagnostics; keep a bounded prefix.
const ERROR_BODY_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct HttpFailure {
    pub class: ErrorClass,
    pub status: Option<u16>,
    pub message: String,
}

impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

pub struct Request {
    pub method: reqwest::Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<Vec<u8>>,
    /// Longest wait for the response head (image generation answers late).
    pub head_timeout: Duration,
    /// Billed per attempt: failures after sending are ambiguous.
    pub paid: bool,
}

impl Http {
    pub fn new(proxy: ProxyMode) -> Result<Self, String> {
        let direct = match proxy {
            ProxyMode::Direct => true,
            ProxyMode::System => false,
            // A dead local proxy (Clash & co.) truncates replies; the toolkit goes direct.
            ProxyMode::Auto => system_proxy_is_loopback(),
        };
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(30))
            .user_agent(concat!("todo-editor/", env!("CARGO_PKG_VERSION")));
        if direct {
            builder = builder.no_proxy();
        }
        builder
            .build()
            .map(|client| Self { client })
            .map_err(|error| error.to_string())
    }

    /// Send and return a 2xx response; anything else is a classified failure.
    pub async fn send(&self, request: Request) -> Result<Response, HttpFailure> {
        let paid = request.paid;
        let mut builder = self.client.request(request.method, &request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = match tokio::time::timeout(request.head_timeout, builder.send()).await {
            Err(_) => {
                return Err(HttpFailure {
                    class: classify_transport(Stage::Sent, paid),
                    status: None,
                    message: format!("no response within {:?}", request.head_timeout),
                })
            }
            Ok(Err(error)) => return Err(transport(&error, paid)),
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let body = read_limited(response, ERROR_BODY_LIMIT).await;
        Err(HttpFailure {
            class: classify_status(status.as_u16(), &body, retry_after, paid),
            status: Some(status.as_u16()),
            message: body.chars().take(400).collect(),
        })
    }
}

/// The next body chunk, failing when nothing arrives for `idle`.
pub async fn next_chunk(
    response: &mut Response,
    idle: Duration,
    paid: bool,
) -> Result<Option<Vec<u8>>, HttpFailure> {
    match tokio::time::timeout(idle, response.chunk()).await {
        Err(_) => Err(HttpFailure {
            class: classify_transport(Stage::Sent, paid),
            status: None,
            message: format!("no data for {idle:?}"),
        }),
        Ok(Err(error)) => Err(transport(&error, paid)),
        Ok(Ok(chunk)) => Ok(chunk.map(|bytes| bytes.to_vec())),
    }
}

/// Whole body with an idle timeout and a size limit.
pub async fn read_body(
    response: &mut Response,
    idle: Duration,
    limit: usize,
    paid: bool,
) -> Result<Vec<u8>, HttpFailure> {
    let mut body = Vec::new();
    while let Some(chunk) = next_chunk(response, idle, paid).await? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(HttpFailure {
                class: ErrorClass::Invalid,
                status: None,
                message: format!("response larger than {limit} bytes"),
            });
        }
    }
    Ok(body)
}

async fn read_limited(mut response: Response, limit: usize) -> String {
    let mut body = Vec::new();
    while let Ok(Ok(Some(chunk))) =
        tokio::time::timeout(Duration::from_secs(10), response.chunk()).await
    {
        body.extend_from_slice(&chunk);
        if body.len() >= limit {
            break;
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

fn transport(error: &reqwest::Error, paid: bool) -> HttpFailure {
    // Connect covers DNS, refused, connect timeout and the TLS handshake.
    let stage = if error.is_connect() || error.is_builder() {
        Stage::Connect
    } else {
        Stage::Sent
    };
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    HttpFailure {
        class: if error.is_builder() {
            ErrorClass::Invalid
        } else {
            classify_transport(stage, paid)
        },
        status: None,
        message,
    }
}

fn is_loopback(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|host| lower.contains(host))
}

/// Proxy environment variables or the Windows (WinINet) user proxy point at this machine.
pub fn system_proxy_is_loopback() -> bool {
    let from_environment = [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .filter_map(|name| std::env::var(name).ok())
    .any(|value| is_loopback(&value));
    from_environment || windows_proxy().is_some_and(|server| is_loopback(&server))
}

#[cfg(windows)]
fn windows_proxy() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
    let enable_name = wide("ProxyEnable");
    let mut enabled = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid NUL-terminated names and an output buffer of `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            enable_name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut enabled as *mut u32).cast(),
            &mut size,
        )
    };
    if status != 0 || enabled == 0 {
        return None;
    }
    let server_name = wide("ProxyServer");
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: as above, with a UTF-16 buffer of `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            server_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

#[cfg(not(windows))]
fn windows_proxy() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn loopback_detection() {
        assert!(super::is_loopback("http://127.0.0.1:7890"));
        assert!(super::is_loopback(
            "http=localhost:1080;https=localhost:1080"
        ));
        assert!(!super::is_loopback("http://proxy.corp:8080"));
    }
}
