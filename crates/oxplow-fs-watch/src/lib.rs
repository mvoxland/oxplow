//! Debounced filesystem watcher.
//!
//! Wraps `notify::RecommendedWatcher` and exposes a single broadcast
//! channel of `WatchEvent`s with built-in debouncing. Reused by
//! oxplow-git for `.git/refs` watching, and (in a future pass) by
//! the analysis pipeline.
