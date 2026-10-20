//! Provider adapters. Each call is one attempt; retrying, pausing and the
//! journal belong to the service.
pub mod huniu;
pub mod vision;
pub mod volc;

use crate::artifacts::{ArtifactError, ArtifactRef};
use crate::net::http::HttpFailure;
use crate::net::ErrorClass;

/// A successful attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub artifact: Option<ArtifactRef>,
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttemptError {
    pub class: ErrorClass,
    pub message: String,
}

impl AttemptError {
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
    /// Local disk trouble: retry, it is not the provider's fault.
    pub fn storage(error: ArtifactError) -> Self {
        Self::new(ErrorClass::Transient, error.to_string())
    }
}

impl From<HttpFailure> for AttemptError {
    fn from(failure: HttpFailure) -> Self {
        let message = failure.to_string();
        Self {
            class: failure.class,
            message,
        }
    }
}
