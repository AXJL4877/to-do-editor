//! A scripted HTTP/1.1 server for fault injection. Each path has a queue of
//! behaviours; the last one repeats. Every request is recorded.
#![allow(dead_code)]
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Answer = Arc<dyn Fn(&Recorded) -> (u16, Vec<u8>) + Send + Sync>;

#[derive(Clone)]
pub enum Behaviour {
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Send the head and the first `cut` body bytes, then close.
    Truncate {
        status: u16,
        body: Vec<u8>,
        cut: usize,
    },
    /// Read the request, then close without answering.
    Drop,
    /// Serve `body`, honouring `Range: bytes=N-` with 206.
    File { body: Vec<u8> },
    /// Answer by inspecting the request.
    Custom(Answer),
}

impl Behaviour {
    pub fn json(status: u16, body: &str) -> Self {
        Self::Respond {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn text(status: u16, body: &str) -> Self {
        Self::Respond {
            status,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct Script(Arc<Mutex<HashMap<String, Vec<Behaviour>>>>);

pub struct FakeServer {
    pub base: String,
    script: Script,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl FakeServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let script = Script::default();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (s, r) = (script.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (s, r) = (s.clone(), r.clone());
                std::thread::spawn(move || serve(stream, &s, &r));
            }
        });
        Self {
            base,
            script,
            requests,
        }
    }

    /// Behaviours for `path`, used in order; the last one repeats.
    pub fn on(&self, path: &str, behaviours: Vec<Behaviour>) {
        self.script
            .0
            .lock()
            .unwrap()
            .insert(path.into(), behaviours);
    }

    pub fn count(&self, method: &str, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method && r.path == path)
            .count()
    }

    pub fn recorded(&self, path: &str) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }
}

fn serve(stream: TcpStream, script: &Script, requests: &Arc<Mutex<Vec<Recorded>>>) {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let recorded = Recorded {
        method,
        path: path.clone(),
        headers,
        body,
    };
    let behaviour = {
        let mut script = script.0.lock().unwrap();
        let queue = script.entry(path).or_default();
        match queue.len() {
            0 => Behaviour::text(404, "no script"),
            1 => queue[0].clone(),
            _ => queue.remove(0),
        }
    };
    requests.lock().unwrap().push(recorded.clone());
    let mut stream = stream;
    let head = |status: u16, extra: &[(String, String)], length: usize| {
        let mut head =
            format!("HTTP/1.1 {status} X\r\nConnection: close\r\nContent-Length: {length}\r\n");
        for (name, value) in extra {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        head
    };
    match behaviour {
        Behaviour::Respond {
            status,
            headers,
            body,
        } => {
            let _ = stream.write_all(head(status, &headers, body.len()).as_bytes());
            let _ = stream.write_all(&body);
        }
        Behaviour::Truncate { status, body, cut } => {
            let _ = stream.write_all(head(status, &[], body.len()).as_bytes());
            let _ = stream.write_all(&body[..cut.min(body.len())]);
            let _ = stream.flush();
        }
        Behaviour::Drop => {}
        Behaviour::File { body } => {
            let start = recorded
                .headers
                .get("range")
                .and_then(|r| r.strip_prefix("bytes="))
                .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
            match start {
                Some(start) if start < body.len() => {
                    let rest = &body[start..];
                    let extra = [(
                        "Content-Range".to_owned(),
                        format!("bytes {start}-{}/{}", body.len() - 1, body.len()),
                    )];
                    let _ = stream.write_all(head(206, &extra, rest.len()).as_bytes());
                    let _ = stream.write_all(rest);
                }
                _ => {
                    let _ = stream.write_all(head(200, &[], body.len()).as_bytes());
                    let _ = stream.write_all(&body);
                }
            }
        }
        Behaviour::Custom(answer) => {
            let (status, body) = answer(&recorded);
            let _ = stream.write_all(head(status, &[], body.len()).as_bytes());
            let _ = stream.write_all(&body);
        }
    }
    let _ = stream.flush();
}
