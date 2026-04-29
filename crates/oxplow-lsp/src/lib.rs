//! LSP proxy crate.
//!
//! Spawns `typescript-language-server` (and any other LSP we add later)
//! as a subprocess, frames JSON-RPC messages, and exposes a
//! `request`/`notify` API plus a server→client message stream.
