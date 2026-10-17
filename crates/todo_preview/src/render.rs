//! Offline frames from `preview_host.exe --render`: the same layer protocol,
//! decoders and shaders as the window preview, drawn offscreen. One frame at
//! a time: send the layer set, read `width × height × 4` BGRA bytes back.
use crate::protocol::{encode_layers, PreviewLayer};
use crate::PreviewError;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct OfflineRenderer {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: std::thread::JoinHandle<String>,
    frame_bytes: usize,
}

impl OfflineRenderer {
    pub fn start(exe: &Path, width: u32, height: u32) -> Result<Self, PreviewError> {
        if !(2..=8192).contains(&width) || !(2..=8192).contains(&height) {
            return Err(PreviewError::Invalid("render size must be 2..=8192"));
        }
        let mut command = Command::new(exe);
        command
            .arg("--render")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(PreviewError::Io)?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut pipe = child.stderr.take().expect("stderr is piped");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        });
        writeln!(stdin, "begin {width} {height}").map_err(PreviewError::Io)?;
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            frame_bytes: width as usize * height as usize * 4,
        })
    }

    /// Draw one frame of `layers` (bottom → top) into `frame`.
    pub fn frame(
        &mut self,
        layers: &[PreviewLayer],
        frame: &mut Vec<u8>,
    ) -> Result<(), PreviewError> {
        let mut command = Vec::new();
        encode_layers(&mut command, layers)?;
        self.stdin.write_all(&command).map_err(PreviewError::Io)?;
        self.stdin.flush().map_err(PreviewError::Io)?;
        frame.resize(self.frame_bytes, 0);
        self.stdout.read_exact(frame).map_err(PreviewError::Io)
    }

    /// End the session; the host's explanation when it failed.
    pub fn finish(mut self) -> Result<(), String> {
        let _ = writeln!(self.stdin, "end");
        drop(self.stdin);
        let status = self.child.wait().map_err(|e| e.to_string())?;
        let stderr = self.stderr.join().unwrap_or_default();
        if status.success() {
            Ok(())
        } else {
            Err(format!("renderer exited with {status}: {}", stderr.trim()))
        }
    }

    /// Stop immediately and collect the host's diagnostics.
    pub fn abort(mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        drop(self.stdin);
        self.stderr.join().unwrap_or_default()
    }
}
