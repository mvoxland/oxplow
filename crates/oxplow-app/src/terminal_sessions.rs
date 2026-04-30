//! Terminal session registry powering the renderer's `TerminalPane`.
//!
//! Each session bridges an xterm.js instance in the renderer to a
//! tmux pane on the host. The renderer talks the same JSON protocol
//! the original Electron build used:
//!
//! - Outgoing (renderer → daemon):
//!   - `{type:"input", bytes:base64}` — user keystrokes
//!   - `{type:"input-binary", bytes:base64}` — binary input (paste)
//!   - `{type:"resize", cols, rows}` — viewport changed
//!   - `{type:"history-page", direction:"up"|"down"}` — page in copy-mode
//!   - `{type:"history-scroll", lines:int}` — scroll N lines (positive = older)
//!   - `{type:"history-exit"}` — leave copy-mode
//!
//! - Incoming (daemon → renderer):
//!   - `{type:"data", bytes:base64}` — bytes from the PTY
//!
//! Implementation: spawn `tmux attach-session -t <pane_target>` via
//! `oxplow_pty::PtyManager`. PTY bytes flow back as `data` events.
//! Resize and copy-mode messages dispatch to the shared `TmuxRunner`.

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
use oxplow_pty::{PaneEvent, PaneId, PtyManager, SpawnRequest};
use oxplow_tmux::{ScrollDirection, TmuxRunner, WindowTarget};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum TerminalSessionError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("pty: {0}")]
    Pty(#[from] oxplow_pty::PtyError),
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("base64: {0}")]
    Base64(String),
}

/// Server-originated frame, tagged with the originating session.
/// Forwarded to the renderer over `terminal:event`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TerminalBridgeEvent {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// JSON-encoded message body using the protocol above.
    pub message: String,
}

struct SessionEntry {
    pane_target: String,
    pane_id: PaneId,
    forwarder: JoinHandle<()>,
}

#[derive(Clone)]
pub struct TerminalSessionRegistry {
    pty: PtyManager,
    tmux: Arc<dyn TmuxRunner>,
    inner: Arc<Mutex<HashMap<String, SessionEntry>>>,
    events_tx: broadcast::Sender<TerminalBridgeEvent>,
}

impl TerminalSessionRegistry {
    pub fn new(pty: PtyManager, tmux: Arc<dyn TmuxRunner>) -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            pty,
            tmux,
            inner: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TerminalBridgeEvent> {
        self.events_tx.subscribe()
    }

    /// Open a new session. Spawns `tmux attach-session -t <pane_target>`
    /// via the PtyManager and starts forwarding bytes back as `data`
    /// messages. Returns the new session_id.
    pub async fn open(
        &self,
        pane_target: String,
        cols: u16,
        rows: u16,
    ) -> Result<String, TerminalSessionError> {
        let req = SpawnRequest {
            command: "tmux".into(),
            args: vec!["attach-session".into(), "-t".into(), pane_target.clone()],
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ],
            cols,
            rows,
        };

        let mut handle = self.pty.spawn_pane(req).await?;
        let pane_id = handle.id.clone();
        let session_id = format!("term-{}", uuid::Uuid::new_v4().simple());

        // Force a tmux repaint so freshly-attached clients see the
        // current pane state immediately even if no new output is
        // produced.
        self.tmux.refresh_clients().await;

        // Spawn a forwarder that pumps PaneEvents → TerminalBridgeEvents.
        let session_id_for_task = session_id.clone();
        let events = self.events_tx.clone();
        let forwarder = tokio::spawn(async move {
            loop {
                match handle.events.recv().await {
                    Ok(PaneEvent::Output(bytes)) => {
                        let msg = serde_json::json!({
                            "type": "data",
                            "bytes": B64.encode(&bytes[..]),
                        })
                        .to_string();
                        let _ = events.send(TerminalBridgeEvent {
                            session_id: session_id_for_task.clone(),
                            message: msg,
                        });
                    }
                    Ok(PaneEvent::Exit { exit_code }) => {
                        let msg = serde_json::json!({
                            "type": "exit",
                            "exitCode": exit_code,
                        })
                        .to_string();
                        let _ = events.send(TerminalBridgeEvent {
                            session_id: session_id_for_task.clone(),
                            message: msg,
                        });
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "terminal forwarder lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("terminal pane channel closed");
                        break;
                    }
                }
            }
        });

        self.inner.lock().await.insert(
            session_id.clone(),
            SessionEntry {
                pane_target,
                pane_id,
                forwarder,
            },
        );
        Ok(session_id)
    }

    /// Dispatch one renderer-issued JSON message.
    pub async fn send(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<(), TerminalSessionError> {
        let parsed: serde_json::Value = serde_json::from_str(message)
            .map_err(|e| TerminalSessionError::InvalidMessage(e.to_string()))?;
        let kind = parsed
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let (pane_id, pane_target) = {
            let map = self.inner.lock().await;
            let entry = map
                .get(session_id)
                .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
            (entry.pane_id.clone(), entry.pane_target.clone())
        };

        match kind.as_str() {
            "input" | "input-binary" => {
                let b64 = parsed
                    .get("bytes")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        TerminalSessionError::InvalidMessage("missing bytes".into())
                    })?;
                let raw = B64
                    .decode(b64)
                    .map_err(|e| TerminalSessionError::Base64(e.to_string()))?;
                self.pty.write(&pane_id, Bytes::from(raw)).await?;
            }
            "resize" => {
                let cols = parsed.get("cols").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                let rows = parsed.get("rows").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                // Reject absurdly-small resizes — see history-mode comment
                // in the original Electron pty-bridge: a hidden xterm can
                // fit-down to two cells and shrink the real tmux window.
                if cols < 20 || rows < 5 {
                    return Ok(());
                }
                self.pty.resize(&pane_id, cols, rows).await?;
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.resize_window(&target, cols, rows).await;
                }
            }
            "history-page" => {
                let dir = match parsed.get("direction").and_then(|v| v.as_str()) {
                    Some("up") => ScrollDirection::Up,
                    Some("down") => ScrollDirection::Down,
                    _ => return Ok(()),
                };
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.copy_mode_page(&target, dir).await;
                }
            }
            "history-scroll" => {
                let lines = parsed.get("lines").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.copy_mode_scroll(&target, lines).await;
                }
            }
            "history-exit" => {
                if let Some(target) = parse_window_target(&pane_target) {
                    self.tmux.exit_copy_mode(&target).await;
                }
            }
            other => {
                debug!(message_type = %other, "unhandled terminal message");
            }
        }
        Ok(())
    }

    pub async fn close(&self, session_id: &str) -> Result<(), TerminalSessionError> {
        let mut map = self.inner.lock().await;
        let entry = map
            .remove(session_id)
            .ok_or_else(|| TerminalSessionError::NotFound(session_id.to_string()))?;
        entry.forwarder.abort();
        let _ = self.pty.kill(&entry.pane_id).await;
        Ok(())
    }
}

/// Pane targets are `"<session>:<window>"`; both sides non-empty.
fn parse_window_target(pane_target: &str) -> Option<WindowTarget> {
    let (session, window) = pane_target.split_once(':')?;
    if session.is_empty() || window.is_empty() {
        return None;
    }
    let session = oxplow_tmux::Session(session.to_string());
    Some(WindowTarget::from_parts(&session, window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_target_round_trip() {
        let t = parse_window_target("oxplow-foo:work").unwrap();
        assert_eq!(t.as_str(), "oxplow-foo:work");
    }

    #[test]
    fn parse_window_target_rejects_malformed() {
        assert!(parse_window_target("nopeartf").is_none());
        assert!(parse_window_target(":x").is_none());
        assert!(parse_window_target("x:").is_none());
    }
}
