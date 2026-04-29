//! Config file load + validation for oxplow.
//!
//! Replaces the TS `src/config/**` modules. Schema validation is
//! enforced at deserialization; errors carry typed variants so the
//! UI can surface them precisely.
