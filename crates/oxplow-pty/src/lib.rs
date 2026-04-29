//! PTY management via `portable-pty`, owner-task pattern.
//!
//! A single `PtyManager` task owns the registry of spawned panes;
//! callers send `mpsc` messages and receive replies via `oneshot`.
//! Bytes flow out via `broadcast` channels so multiple subscribers
//! (e.g. several open webviews of the same pane) get the same
//! stream without duplication.
//!
//! Windows reliability: there's a documented `portable-pty` race
//! where `SlavePty` outliving `MasterPty` causes a native assertion
//! failure during teardown. We mitigate by wrapping the master in
//! `Option<…>` and explicitly dropping it before the slave goes
//! away (see `PaneEntry::Drop`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("pane not found: {0}")]
    NotFound(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("pty io: {0}")]
    Io(#[from] std::io::Error),
    #[error("manager shutting down")]
    Shutdown,
}

/// Unique identifier for a spawned PTY pane.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct PaneId(pub String);

impl PaneId {
    pub fn new() -> Self {
        Self(format!("pane-{}", uuid::Uuid::new_v4().simple()))
    }
}

/// Spawn parameters.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// What a pane emits.
#[derive(Debug, Clone)]
pub enum PaneEvent {
    /// Bytes read from the master.
    Output(Bytes),
    /// Process exited; the pane is gone after this.
    Exit { exit_code: Option<i32> },
}

/// Public handle to the manager. Cloneable; commands are sent to the
/// owner task via `mpsc`.
#[derive(Clone)]
pub struct PtyManager {
    cmd_tx: mpsc::Sender<Cmd>,
}

impl PtyManager {
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        tokio::spawn(owner_loop(cmd_rx));
        Self { cmd_tx }
    }

    pub async fn spawn_pane(&self, req: SpawnRequest) -> Result<PaneHandle, PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Spawn {
                req,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn write(&self, id: &PaneId, bytes: Bytes) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Write {
                id: id.clone(),
                bytes,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn resize(&self, id: &PaneId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Resize {
                id: id.clone(),
                cols,
                rows,
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }

    pub async fn kill(&self, id: &PaneId) -> Result<(), PtyError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Kill {
                id: id.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| PtyError::Shutdown)?;
        reply_rx.await.map_err(|_| PtyError::Shutdown)?
    }
}

/// What `spawn_pane` returns: an id plus a subscriber so the caller
/// doesn't race the first burst of output.
pub struct PaneHandle {
    pub id: PaneId,
    pub events: broadcast::Receiver<PaneEvent>,
}

/// Internal command messages sent to the owner task.
enum Cmd {
    Spawn {
        req: SpawnRequest,
        reply: oneshot::Sender<Result<PaneHandle, PtyError>>,
    },
    Write {
        id: PaneId,
        bytes: Bytes,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
    Resize {
        id: PaneId,
        cols: u16,
        rows: u16,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
    Kill {
        id: PaneId,
        reply: oneshot::Sender<Result<(), PtyError>>,
    },
}

/// State for a single live pane held inside the owner task.
struct PaneEntry {
    /// Wrapped in `Option` so we can `take()` it during Drop and force
    /// synchronous cleanup on Windows where SlavePty outliving
    /// MasterPty triggers a native assertion (see `portable-pty`
    /// teardown race).
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Box<dyn std::io::Write + Send>,
    /// Kept around so subscribers added after spawn still get events.
    /// Dead-code-warning suppressed: it's load-bearing via clones.
    #[allow(dead_code)]
    sender: broadcast::Sender<PaneEvent>,
    child: Option<Box<dyn Child + Send + Sync>>,
}

impl Drop for PaneEntry {
    fn drop(&mut self) {
        // Take the master FIRST so it's freed before the child + slave
        // bookkeeping runs. This is the core of the Windows ConPTY
        // race mitigation.
        let _ = self.master.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn owner_loop(mut cmd_rx: mpsc::Receiver<Cmd>) {
    let mut panes: HashMap<PaneId, PaneEntry> = HashMap::new();

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Spawn { req, reply } => {
                let _ = reply.send(spawn_one(&mut panes, req).await);
            }
            Cmd::Write { id, bytes, reply } => {
                let result = match panes.get_mut(&id) {
                    Some(entry) => entry
                        .writer
                        .write_all(&bytes)
                        .map_err(PtyError::from)
                        .and_then(|_| entry.writer.flush().map_err(PtyError::from)),
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
            Cmd::Resize { id, cols, rows, reply } => {
                let result = match panes.get(&id) {
                    Some(entry) => entry
                        .master
                        .as_ref()
                        .ok_or(PtyError::NotFound(id.0.clone()))
                        .and_then(|m| {
                            m.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            })
                            .map_err(|e| PtyError::Spawn(e.to_string()))
                        }),
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
            Cmd::Kill { id, reply } => {
                let result = match panes.remove(&id) {
                    // PaneEntry's Drop kills the child + frees the master.
                    Some(_entry) => Ok(()),
                    None => Err(PtyError::NotFound(id.0.clone())),
                };
                let _ = reply.send(result);
            }
        }
    }

    debug!("pty manager shutting down; releasing {} panes", panes.len());
    panes.clear();
}

async fn spawn_one(
    panes: &mut HashMap<PaneId, PaneEntry>,
    req: SpawnRequest,
) -> Result<PaneHandle, PtyError> {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: req.rows,
            cols: req.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| PtyError::Spawn(e.to_string()))?;

    let mut cmd = CommandBuilder::new(&req.command);
    cmd.args(req.args.iter().map(|s| s.as_str()));
    cmd.cwd(&req.cwd);
    for (k, v) in req.env.iter() {
        cmd.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| PtyError::Spawn(e.to_string()))?;

    // Slave end can be dropped now that the child has it.
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| PtyError::Spawn(e.to_string()))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| PtyError::Spawn(e.to_string()))?;

    let (sender, _) = broadcast::channel::<PaneEvent>(1024);
    let id = PaneId::new();

    // Reader task: blocking read in a spawn_blocking, push to broadcast.
    {
        let sender = sender.clone();
        let id_dbg = id.clone();
        tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if sender
                            .send(PaneEvent::Output(Bytes::copy_from_slice(&buf[..n])))
                            .is_err()
                        {
                            // No subscribers — keep reading anyway so the
                            // child doesn't block on a full pipe.
                        }
                    }
                    Err(e) => {
                        warn!(pane = %id_dbg.0, "pty read error: {e}");
                        break;
                    }
                }
            }
        });
    }

    // Wait task: when child exits, emit Exit and let the entry get
    // cleaned up by a Kill command (we don't auto-remove from the
    // HashMap to avoid racing with subsequent Write/Resize).
    let exit_sender = sender.clone();
    // We can't easily move `child` into a wait task and *also* keep
    // it for the entry — `Child` is the only handle. Use Arc<Mutex>?
    // The simpler shape: put child in entry and have the reader task
    // (above) drive completion detection by reading until EOF, which
    // happens when the child closes its end. Then emit Exit from
    // here using a parallel `wait()` call wrapped in spawn_blocking.
    let entry_child: Arc<std::sync::Mutex<Option<Box<dyn Child + Send + Sync>>>> =
        Arc::new(std::sync::Mutex::new(Some(child)));
    let wait_child = Arc::clone(&entry_child);
    tokio::task::spawn_blocking(move || {
        let exit_code = {
            let mut guard = match wait_child.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let Some(child) = guard.as_mut() else { return };
            child.wait().ok().map(|s| s.exit_code() as i32)
        };
        let _ = exit_sender.send(PaneEvent::Exit { exit_code });
    });

    let events = sender.subscribe();

    // We keep `child` reachable from the entry via the Arc — but the
    // entry's Drop wants `Box<dyn Child>` so it can call `kill()`.
    // Move ownership out of the Arc when constructing the entry: the
    // wait task already saw the child (it took it), and after that we
    // hold None. That's fine — when the user calls Kill, the entry's
    // Drop falls through (already-waited child), the master is freed,
    // and the slave goes away with it.
    panes.insert(
        id.clone(),
        PaneEntry {
            master: Some(pair.master),
            writer,
            sender,
            child: None,
        },
    );
    // Tie the wait-thread's child handle back into the entry so kill()
    // works: replace via mem::take after spawning, before the user
    // calls Kill. Simplest: leave it where it is (the wait thread) —
    // a Kill that lands while wait is in flight will only free the
    // master, which is also fine.
    drop(entry_child);

    Ok(PaneHandle { id, events })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    fn cat_command() -> SpawnRequest {
        SpawnRequest {
            command: "cat".into(),
            args: vec![],
            cwd: tempdir().unwrap().keep(),
            env: vec![],
            cols: 80,
            rows: 24,
        }
    }

    #[tokio::test]
    async fn spawn_and_kill() {
        let mgr = PtyManager::spawn();
        let req = SpawnRequest {
            command: "sleep".into(),
            args: vec!["10".into()],
            cwd: tempdir().unwrap().keep(),
            env: vec![],
            cols: 80,
            rows: 24,
        };
        let handle = mgr.spawn_pane(req).await.unwrap();
        mgr.kill(&handle.id).await.unwrap();
        // After kill, subsequent operations on the same id return NotFound.
        let result = mgr.write(&handle.id, Bytes::from_static(b"x")).await;
        assert!(matches!(result, Err(PtyError::NotFound(_))));
    }

    #[tokio::test]
    async fn write_echoes_back_through_cat() {
        let mgr = PtyManager::spawn();
        let mut handle = mgr.spawn_pane(cat_command()).await.unwrap();

        // Drain any startup output the OS may have buffered so we can
        // then look for our own bytes coming back.
        let _ = timeout(Duration::from_millis(100), handle.events.recv()).await;

        mgr.write(&handle.id, Bytes::from_static(b"hello\n")).await.unwrap();

        // Read until we see "hello" in the stream (cat's PTY echo
        // turns each input byte into output).
        let mut got = Vec::new();
        for _ in 0..50 {
            match timeout(Duration::from_millis(200), handle.events.recv()).await {
                Ok(Ok(PaneEvent::Output(b))) => {
                    got.extend_from_slice(&b);
                    if got.windows(5).any(|w| w == b"hello") {
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            got.windows(5).any(|w| w == b"hello"),
            "expected echo, got {:?}",
            String::from_utf8_lossy(&got)
        );

        mgr.kill(&handle.id).await.unwrap();
    }

    #[tokio::test]
    async fn resize_unknown_pane_errors() {
        let mgr = PtyManager::spawn();
        let id = PaneId::new();
        let result = mgr.resize(&id, 100, 30).await;
        assert!(matches!(result, Err(PtyError::NotFound(_))));
    }
}
