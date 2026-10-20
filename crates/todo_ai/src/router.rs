//! Capability-first AI routing.
//!
//! Modules submit a typed capability request. The router owns the model
//! binding and delegates retries, journaling, artifacts and provider-specific
//! wire formats to [`AiService`]. Provider names stay below this boundary.

use crate::{AiEvent, AiService, ArtifactRef, Recovered, Secret, ServiceSettings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use todo_production::{Capability, Provider, TaskKey, TaskRequest, TaskSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelBinding {
    pub model: String,
    pub provider: Provider,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityBindings {
    pub models: HashMap<Capability, ModelBinding>,
}

impl CapabilityBindings {
    pub fn model(&self, capability: Capability) -> Option<&str> {
        self.models
            .get(&capability)
            .map(|binding| binding.model.as_str())
    }

    pub fn provider_map(&self) -> HashMap<Capability, Provider> {
        self.models
            .iter()
            .map(|(capability, binding)| (*capability, binding.provider))
            .collect()
    }
}

/// Application-facing AI gateway. It deliberately has no provider-specific
/// submit methods; extensions and the built-in Agent use capability requests.
pub struct AiRouter {
    service: AiService,
    bindings: CapabilityBindings,
}

impl AiRouter {
    pub fn start(
        mut settings: ServiceSettings,
        bindings: CapabilityBindings,
    ) -> Result<(Self, Vec<Recovered>), String> {
        settings.bindings = bindings.provider_map();
        settings.models = bindings
            .models
            .iter()
            .map(|(capability, binding)| (*capability, binding.model.clone()))
            .collect();
        let (service, recovered) = AiService::start(settings)?;
        Ok((Self { service, bindings }, recovered))
    }

    /// Submit a typed capability request. An empty model is resolved by the
    /// executor immediately before the provider call, so the task key stays
    /// stable for the production planner and extension worker.
    pub fn submit(&self, request: TaskRequest) -> TaskKey {
        self.service.submit(self.prepare(request))
    }

    /// Wrap a capability request as a journal task snapshot. Pictures of a
    /// multimodal request are fingerprinted by content.
    pub fn prepare(&self, mut request: TaskRequest) -> TaskSpec {
        match &mut request {
            TaskRequest::Multimodal(value) => {
                value.digests = crate::providers::vision::media_digests(&value.media);
            }
            TaskRequest::Image(value) if !value.references.is_empty() => {
                value.reference_digests =
                    crate::providers::vision::media_digests(&value.references);
            }
            _ => {}
        }
        TaskSpec { request }
    }

    pub fn submit_spec(&self, spec: TaskSpec) -> TaskKey {
        self.service.submit(spec)
    }

    pub fn binding(&self, capability: Capability) -> Option<&ModelBinding> {
        self.bindings.models.get(&capability)
    }

    pub fn set_key(&self, provider: Provider, key: Option<Secret>) {
        self.service.set_key(provider, key);
    }
    pub fn cancel(&self, key: &TaskKey) {
        self.service.cancel(key);
    }
    pub fn retry(&self, key: &TaskKey) {
        self.service.retry(key);
    }
    /// Discover models for a capability using its bound provider. A provider
    /// may return a mixed catalog; the UI filters it by `ModelCapability`.
    pub fn list_models(&self, capability: Capability) -> u64 {
        let provider = self
            .binding(capability)
            .map(|binding| binding.provider)
            .unwrap_or(Provider::Huniu);
        self.service.list_models(provider, capability)
    }
    pub fn poll(&self) -> Option<AiEvent> {
        self.service.poll()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Option<AiEvent> {
        self.service.recv_timeout(timeout)
    }
    pub fn artifact_path(&self, artifact: &ArtifactRef) -> PathBuf {
        self.service.artifact_path(artifact)
    }
}
