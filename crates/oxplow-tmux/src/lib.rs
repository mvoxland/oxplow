//! tmux command builder.
//!
//! Trait-based wrapper over `tokio::process::Command` invocations
//! against the tmux CLI. Encodes the "window-size manual" + placeholder
//! window invariants from the existing TS implementation
//! (`src/terminal/tmux.ts`).
//!
//! The trait `TmuxRunner` exists so `oxplow-runtime` tests can mock
//! the tmux surface; the real impl `SystemTmux` shells out to the
//! `tmux` binary on PATH.

use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum TmuxError {
    #[error("tmux command failed: {0}")]
    CommandFailed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A tmux session name. Newtype so we don't accidentally pass an
/// arbitrary string into a session-typed slot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Session(pub String);

impl Session {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A tmux window target of the form `session:window`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTarget(pub String);

impl WindowTarget {
    pub fn from_parts(session: &Session, window: &str) -> Self {
        Self(format!("{}:{}", session.0, window))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn split(&self) -> (&str, &str) {
        match self.0.split_once(':') {
            Some((s, w)) => (s, w),
            None => (&self.0, ""),
        }
    }
}

#[async_trait]
pub trait TmuxRunner: Send + Sync {
    async fn has_session(&self, session: &Session) -> bool;
    async fn ensure_session(&self, session: &Session, cwd: &Path) -> Result<(), TmuxError>;
    async fn has_window(&self, target: &WindowTarget) -> bool;
    async fn ensure_window(
        &self,
        target: &WindowTarget,
        cwd: &Path,
        command: &str,
        cols: u16,
        rows: u16,
        launcher_signature: Option<&str>,
    ) -> Result<bool, TmuxError>;
    async fn resize_window(&self, target: &WindowTarget, cols: u16, rows: u16);
    async fn kill_window(&self, target: &WindowTarget);
    async fn kill_session(&self, session: &Session);
    async fn list_windows(&self, session: &Session) -> Vec<String>;
    async fn capture_pane_history(&self, target: &WindowTarget, line_count: u32) -> String;
    async fn refresh_clients(&self);
    /// Page through the scrollback (`up` for older lines, `down` for newer).
    /// Enters copy-mode if not already in it. No-op if tmux fails.
    async fn copy_mode_page(&self, target: &WindowTarget, direction: ScrollDirection);
    /// Scroll by `lines` (positive = older, negative = newer).
    async fn copy_mode_scroll(&self, target: &WindowTarget, lines: i32);
    /// Leave copy-mode and snap to the live tail.
    async fn exit_copy_mode(&self, target: &WindowTarget);
}

/// Direction for `copy_mode_page`.
#[derive(Debug, Clone, Copy)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Default)]
pub struct SystemTmux;

impl SystemTmux {
    pub fn new() -> Self {
        Self
    }

    async fn run(args: &[&str]) -> Result<String, TmuxError> {
        let out = Command::new("tmux")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await?;
        if !out.status.success() {
            return Err(TmuxError::CommandFailed(format!(
                "tmux {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn run_quiet(args: &[&str]) -> bool {
        Command::new("tmux")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl TmuxRunner for SystemTmux {
    async fn has_session(&self, session: &Session) -> bool {
        Self::run_quiet(&["has-session", "-t", session.as_str()]).await
    }

    async fn ensure_session(&self, session: &Session, cwd: &Path) -> Result<(), TmuxError> {
        if self.has_session(session).await {
            return Ok(());
        }
        let cwd_s = cwd.to_string_lossy();
        // Placeholder window keeps the session alive until a real
        // window is created; killed once any real window exists.
        Self::run(&[
            "new-session",
            "-d",
            "-s",
            session.as_str(),
            "-c",
            &cwd_s,
            "-n",
            "__placeholder__",
        ])
        .await?;
        // Manual window-size: we drive resizes explicitly so the agent's
        // output isn't re-wrapped under a transient client size.
        let _ = Self::run(&[
            "set-option",
            "-t",
            session.as_str(),
            "window-size",
            "manual",
        ])
        .await;
        Ok(())
    }

    async fn has_window(&self, target: &WindowTarget) -> bool {
        let (session, window) = target.split();
        let Ok(out) = Self::run(&["list-windows", "-t", session, "-F", "#{window_name}"]).await
        else {
            return false;
        };
        out.lines().any(|line| line == window)
    }

    async fn ensure_window(
        &self,
        target: &WindowTarget,
        cwd: &Path,
        command: &str,
        cols: u16,
        rows: u16,
        launcher_signature: Option<&str>,
    ) -> Result<bool, TmuxError> {
        if self.has_window(target).await {
            // If the launcher signature differs, the window is stale —
            // kill it so we re-create with the new command.
            let stale = match launcher_signature {
                Some(sig) => read_window_signature(target).await.as_deref() != Some(sig),
                None => false,
            };
            if stale {
                self.kill_window(target).await;
            } else {
                self.resize_window(target, cols, rows).await;
                return Ok(false);
            }
        }
        let (session, window) = target.split();
        let size = format!("{cols}x{rows}");
        let _ = Self::run(&["set-option", "-t", session, "default-size", &size]).await;
        let cwd_s = cwd.to_string_lossy();
        Self::run(&[
            "new-window",
            "-d",
            "-t",
            session,
            "-n",
            window,
            "-c",
            &cwd_s,
            command,
        ])
        .await?;
        if let Some(sig) = launcher_signature {
            let _ = Self::run(&[
                "set-option",
                "-w",
                "-t",
                target.as_str(),
                "@oxplow_launcher_signature",
                sig,
            ])
            .await;
        }
        self.resize_window(target, cols, rows).await;
        Ok(true)
    }

    async fn resize_window(&self, target: &WindowTarget, cols: u16, rows: u16) {
        if cols < 2 || rows < 2 {
            return;
        }
        let cols_s = cols.to_string();
        let rows_s = rows.to_string();
        let _ = Self::run(&[
            "resize-window",
            "-t",
            target.as_str(),
            "-x",
            &cols_s,
            "-y",
            &rows_s,
        ])
        .await;
    }

    async fn kill_window(&self, target: &WindowTarget) {
        let _ = Self::run(&["kill-window", "-t", target.as_str()]).await;
    }

    async fn kill_session(&self, session: &Session) {
        let _ = Self::run(&["kill-session", "-t", session.as_str()]).await;
    }

    async fn list_windows(&self, session: &Session) -> Vec<String> {
        let Ok(out) = Self::run(&[
            "list-windows",
            "-t",
            session.as_str(),
            "-F",
            "#{window_name}",
        ])
        .await
        else {
            return Vec::new();
        };
        out.lines()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    async fn capture_pane_history(&self, target: &WindowTarget, line_count: u32) -> String {
        let lc = format!("-{line_count}");
        Self::run(&[
            "capture-pane",
            "-p",
            "-e",
            "-J",
            "-S",
            &lc,
            "-t",
            target.as_str(),
        ])
        .await
        .unwrap_or_default()
    }

    async fn refresh_clients(&self) {
        let _ = Self::run(&["refresh-client"]).await;
    }

    async fn copy_mode_page(&self, target: &WindowTarget, direction: ScrollDirection) {
        // Enter copy-mode (idempotent) then page.
        let _ = Self::run_quiet(&["copy-mode", "-t", target.as_str()]).await;
        let key = match direction {
            ScrollDirection::Up => "PageUp",
            ScrollDirection::Down => "PageDown",
        };
        let _ = Self::run_quiet(&["send-keys", "-t", target.as_str(), "-X", "-N", "1", key]).await;
    }

    async fn copy_mode_scroll(&self, target: &WindowTarget, lines: i32) {
        let _ = Self::run_quiet(&["copy-mode", "-t", target.as_str()]).await;
        if lines == 0 {
            return;
        }
        let (cmd, count) = if lines > 0 {
            ("scroll-up", lines)
        } else {
            ("scroll-down", -lines)
        };
        let count_s = count.to_string();
        let _ = Self::run_quiet(&[
            "send-keys",
            "-t",
            target.as_str(),
            "-X",
            "-N",
            &count_s,
            cmd,
        ])
        .await;
    }

    async fn exit_copy_mode(&self, target: &WindowTarget) {
        let _ = Self::run_quiet(&["send-keys", "-t", target.as_str(), "-X", "cancel"]).await;
    }
}

async fn read_window_signature(target: &WindowTarget) -> Option<String> {
    let out = SystemTmux::run(&[
        "show-options",
        "-w",
        "-v",
        "-t",
        target.as_str(),
        "@oxplow_launcher_signature",
    ])
    .await
    .ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Detect whether `tmux` is on PATH. Tests gate themselves on this so
/// the suite passes on CI runners without tmux installed.
pub fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn unique_session() -> Session {
        Session(format!("oxplow-test-{}", Uuid::new_v4().simple()))
    }

    #[tokio::test]
    async fn target_splits_correctly() {
        let s = Session("sess".into());
        let t = WindowTarget::from_parts(&s, "win");
        assert_eq!(t.as_str(), "sess:win");
        assert_eq!(t.split(), ("sess", "win"));
    }

    #[tokio::test]
    async fn ensure_session_creates_then_idempotent() {
        if !tmux_available() {
            return;
        }
        let tmux = SystemTmux::new();
        let dir = tempdir().unwrap();
        let s = unique_session();
        // Cleanup paranoia: kill in case a prior run left it.
        tmux.kill_session(&s).await;

        assert!(!tmux.has_session(&s).await);
        tmux.ensure_session(&s, dir.path()).await.unwrap();
        assert!(tmux.has_session(&s).await);
        // Second call is a no-op.
        tmux.ensure_session(&s, dir.path()).await.unwrap();
        assert!(tmux.has_session(&s).await);

        tmux.kill_session(&s).await;
        assert!(!tmux.has_session(&s).await);
    }

    #[tokio::test]
    async fn list_windows_includes_placeholder_after_ensure() {
        if !tmux_available() {
            return;
        }
        let tmux = SystemTmux::new();
        let dir = tempdir().unwrap();
        let s = unique_session();
        tmux.kill_session(&s).await;
        tmux.ensure_session(&s, dir.path()).await.unwrap();
        let windows = tmux.list_windows(&s).await;
        assert!(windows.iter().any(|w| w == "__placeholder__"));
        tmux.kill_session(&s).await;
    }
}
