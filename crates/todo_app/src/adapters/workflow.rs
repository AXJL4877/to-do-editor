//! Host services for the workflow runner: one synchronous call per node.
//!
//! Chat / image / speech / subtitles go through [`ServiceBridge`] (durable AI
//! journal, artifact materialization, speech timelines); vision goes straight
//! to the [`AiRouter`] because `multimodal` is a task, not a worker service.
use std::path::PathBuf;
use std::time::{Duration, Instant};
use todo_ai::{AiEvent, AiRouter};
use todo_workflow::workflow::protocol::{ServiceCall, SpeechCall, SubtitlesCall};
use todo_production::tasks::{MultimodalRequest, TaskRequest};
use todo_workflow::runner::{run, RunInput, RunReport, Services};
use todo_workflow::Workflow;

use super::production::bridge::ServiceBridge;
use todo_workflow::RunId;

/// Default ceiling for one node's service call (image generation is the slow one).
pub const NODE_TIMEOUT: Duration = Duration::from_secs(300);

pub struct HostServices<'a> {
    bridge: &'a mut ServiceBridge,
    ai: &'a AiRouter,
    run: RunId,
    next: u32,
    timeout: Duration,
}

impl<'a> HostServices<'a> {
    pub fn new(bridge: &'a mut ServiceBridge, ai: &'a AiRouter, run: RunId) -> Self {
        Self {
            bridge,
            ai,
            run,
            next: 0,
            timeout: NODE_TIMEOUT,
        }
    }

    fn call(&mut self, service: ServiceCall) -> Result<serde_json::Value, String> {
        self.next += 1;
        let id = format!("workflow-{}", self.next);
        self.bridge
            .call_blocking(self.ai, self.run, id, service, self.timeout)
    }

    /// Vision is a task request; wait for its terminal event here.
    fn vision_call(&mut self, prompt: String, images: Vec<PathBuf>) -> Result<serde_json::Value, String> {
        let media: Vec<String> = images
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let request = MultimodalRequest {
            model: String::new(),
            prompt,
            digests: todo_ai::providers::vision::media_digests(&media),
            media,
            json: false,
        };
        let key = self.ai.submit(TaskRequest::Multimodal(request));
        let deadline = Instant::now() + self.timeout;
        loop {
            if Instant::now() >= deadline {
                self.ai.cancel(&key);
                return Err("审图超时".into());
            }
            let Some(event) = self.ai.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            match &event {
                AiEvent::Done { key: done, output, .. } if *done == key => {
                    let text = output
                        .as_ref()
                        .and_then(|value| value.get("text"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    return Ok(serde_json::json!({ "text": text }));
                }
                AiEvent::Failed { key: failed, message, .. } if *failed == key => {
                    return Err(message.clone());
                }
                AiEvent::Cancelled { key: cancelled } if *cancelled == key => {
                    return Err("审图已取消".into());
                }
                other => self.bridge.on_ai_event(self.ai, other),
            }
        }
    }
}

impl Services for HostServices<'_> {
    fn chat(
        &mut self,
        call: todo_workflow::workflow::protocol::ChatCall,
    ) -> Result<serde_json::Value, String> {
        self.call(ServiceCall::Chat(call))
    }

    fn image(
        &mut self,
        call: todo_workflow::workflow::protocol::ImageCall,
    ) -> Result<serde_json::Value, String> {
        self.call(ServiceCall::Image(call))
    }

    fn vision(&mut self, prompt: String, images: Vec<PathBuf>) -> Result<serde_json::Value, String> {
        self.vision_call(prompt, images)
    }

    fn speech(&mut self, call: SpeechCall) -> Result<serde_json::Value, String> {
        self.call(ServiceCall::Speech(call))
    }

    fn subtitles(&mut self, call: SubtitlesCall) -> Result<serde_json::Value, String> {
        self.call(ServiceCall::Subtitles(call))
    }
}

/// Execute a workflow in its own run directory; the caller keeps the report.
pub fn run_workflow(
    workflow: &Workflow,
    package_dir: &std::path::Path,
    workdir: PathBuf,
    bridge: &mut ServiceBridge,
    ai: &AiRouter,
    run_id: RunId,
    script: String,
) -> RunReport {
    std::fs::create_dir_all(&workdir).ok();
    bridge.register_run(run_id, workdir);
    let mut services = HostServices::new(bridge, ai, run_id);
    run(
        &workflow.graph,
        package_dir,
        &mut services,
        &RunInput { script, at: Some(0.0) },
        &mut |_items| {},
    )
}
