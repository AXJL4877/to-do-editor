//! A long-running child spoken to line by line (extension workers): stdin for
//! requests (written on its own thread, so a child that stops reading never
//! blocks the caller), stdout lines forwarded to a channel, stderr kept as a
//! bounded tail for diagnostics. Dropping it kills and reaps the child.
use crate::{hide_window, Job, ProcessError};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// Longest stdout line accepted; longer output ends the stream.
pub const MAX_LINE: usize = 16 * 1024 * 1024;
const STDERR_TAIL: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Line(String),
    /// stdout closed (the child exited or closed it).
    Closed,
    /// A line exceeded [`MAX_LINE`] or was not UTF-8.
    Invalid(String),
}

pub struct Duplex {
    tool: PathBuf,
    child: Child,
    stdin: Option<Sender<String>>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
}

impl Duplex {
    /// Spawn `command`; every stdout line is sent as `wrap(Output)`.
    pub fn spawn<T: Send + 'static>(
        command: Command,
        sender: Sender<T>,
        wrap: impl Fn(Output) -> T + Send + 'static,
    ) -> Result<Self, ProcessError> {
        Self::spawn_in(command, None, sender, wrap)
    }

    /// Spawn a worker and optionally assign it to a Windows Job Object before
    /// any worker thread can observe its pipes.
    pub fn spawn_in<T: Send + 'static>(
        mut command: Command,
        job: Option<&Job>,
        sender: Sender<T>,
        wrap: impl Fn(Output) -> T + Send + 'static,
    ) -> Result<Self, ProcessError> {
        let tool = PathBuf::from(command.get_program());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut command);
        let mut child = command.spawn().map_err(|e| ProcessError::Spawn {
            tool: tool.clone(),
            message: e.to_string(),
        })?;
        if let Some(job) = job {
            if let Err(error) = job.assign(&child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr_pipe = child.stderr.take().expect("stderr is piped");
        let stderr = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
        let tail = stderr.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut buffer = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.extend(&buffer[..n]);
                let excess = tail.len().saturating_sub(STDERR_TAIL);
                tail.drain(..excess);
            }
        });
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                let read = (&mut reader)
                    .take(MAX_LINE as u64 + 1)
                    .read_until(b'\n', &mut line);
                match read {
                    Ok(0) | Err(_) => {
                        let _ = sender.send(wrap(Output::Closed));
                        return;
                    }
                    Ok(_) if line.len() > MAX_LINE => {
                        let _ = sender.send(wrap(Output::Invalid(format!(
                            "output line longer than {MAX_LINE} bytes"
                        ))));
                        return;
                    }
                    Ok(_) => {
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        let message = match String::from_utf8(std::mem::take(&mut line)) {
                            Ok(text) => Output::Line(text),
                            Err(_) => Output::Invalid("output is not UTF-8".into()),
                        };
                        let invalid = matches!(message, Output::Invalid(_));
                        if sender.send(wrap(message)).is_err() || invalid {
                            return;
                        }
                    }
                }
            }
        });
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let (lines, queue) = mpsc::channel::<String>();
        thread::spawn(move || {
            for mut line in queue {
                line.push('\n');
                if pipe
                    .write_all(line.as_bytes())
                    .and_then(|()| pipe.flush())
                    .is_err()
                {
                    return;
                }
            }
        });
        Ok(Self {
            tool,
            stdin: Some(lines),
            child,
            stderr,
        })
    }

    /// Queue one line for the child's stdin; never blocks. Fails once the
    /// child is gone.
    pub fn send_line(&mut self, line: &str) -> Result<(), ProcessError> {
        if self.exited() {
            self.stdin = None;
        }
        let closed = || ProcessError::Output(format!("{}: stdin is closed", self.tool.display()));
        let stdin = self.stdin.as_ref().ok_or_else(closed)?;
        stdin.send(line.to_owned()).map_err(|_| closed())
    }

    /// The last few kilobytes of stderr.
    pub fn stderr_tail(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let bytes: Vec<u8> = tail.iter().copied().collect();
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }

    /// Exited already?
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Duplex {
    fn drop(&mut self) {
        self.kill();
    }
}
