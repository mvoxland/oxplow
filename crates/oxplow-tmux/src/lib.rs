//! tmux command builder.
//!
//! Trait-based wrapper over `tokio::process::Command` invocations
//! against the tmux CLI. Encodes the "window-size manual" + placeholder
//! window invariants from the existing TS implementation.
