//! Public desktop adapters used by the application binary and the end-to-end
//! production harness. UI composition remains in `main.rs`; adapters are
//! reusable so online production checks exercise the same host and bridge as
//! the shipped app.
pub mod adapters;
