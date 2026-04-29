//! PTY management via `portable-pty`, owner-task pattern.
//!
//! Single `PtyManager` task owns all spawned panes; commands send
//! `mpsc` messages and receive replies via `oneshot`. Bytes flow out
//! via `broadcast` channels so every connected webview gets the same
//! stream without duplication.
//!
//! Windows: explicit `Drop` impl mitigates the documented `portable-pty`
//! ConPTY teardown race (SlavePty outlives MasterPty).
