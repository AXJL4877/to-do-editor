//! External AI services: configuration, secrets, resilient networking, the
//! durable task journal and the verified artifact store. Infrastructure only:
//! what to request comes from `todo_production`.
pub mod artifacts;
pub mod config;
pub mod journal;
pub mod models;
pub mod net;
pub mod providers;
pub mod router;
pub mod secrets;
pub mod service;

pub use artifacts::{ArtifactError, ArtifactRef, ArtifactStore};
pub use config::{AiConfig, ConfigError, ProxyMode};
pub use journal::{Journal, JournalError, Recovered, TaskClaim, TaskRecord, TaskState};
pub use models::{classify_models, ModelCapability, ModelInfo};
pub use net::{BreakerPolicy, RetryPolicy};
pub use router::{AiRouter, CapabilityBindings, ModelBinding};
pub use secrets::{Secret, SecretError, SecretSource, SecretStore, Secrets};
pub use service::default_paths;
pub use service::{AiEvent, AiService, Attention, LanguageEndpoint, ServiceSettings, WaitReason};
