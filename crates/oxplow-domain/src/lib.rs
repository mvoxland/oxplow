//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, work items, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod error;
pub mod ids;
pub mod time;

pub use error::DomainError;
