//! Resilient networking: failure classes, retry decisions and circuit breaking.
pub mod breaker;
pub mod classify;
pub mod http;
pub mod retry;

pub use breaker::{Breaker, BreakerPolicy, Permit};
pub use classify::{classify_status, classify_transport, parse_retry_after, ErrorClass, Stage};
pub use retry::{deadline, Attempts, Decision, RetryPolicy};
