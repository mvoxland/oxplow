//! Lightweight self-diagnostics.
//!
//! Spawns a tokio task that, once a minute, samples three numbers and
//! emits them at `tracing::info`:
//!
//! - **RSS** (resident-set size, KB) — does the process leak memory
//!   over a long session?
//! - **open fds** — proxy for "are watchers / sockets / files
//!   piling up". A steady-state count means the process is releasing
//!   handles cleanly; monotonic growth is the signal we care about.
//! - **streams** — number of stream rows, which equals the number of
//!   per-stream `WorkspaceWatchRegistry` watcher pairs alive.
//!
//! Cheap by construction: one `ps` exec, one `read_dir("/dev/fd")`,
//! one `streams.list()` per minute. No new crate deps.
//!
//! Why this exists: a user reported a system-wide hang and wondered
//! whether oxplow's watchers were leaking handles. With this in place,
//! the next incident has data — grep `tracing` output for `diagnostics`
//! and look at the trend.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::stores::StreamStore;
use tracing::{info, warn};

/// How often to sample. Keep it long — these numbers move slowly and
/// we don't want diagnostics noise in the log.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn the diagnostics loop on the current tokio runtime. Returns
/// immediately; the loop runs until the process exits.
pub fn spawn(streams: Arc<dyn StreamStore>) {
    tokio::spawn(async move {
        // Stagger the first sample so it doesn't race with boot.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            sample_once(&streams).await;
            tokio::time::sleep(SAMPLE_INTERVAL).await;
        }
    });
}

async fn sample_once(streams: &Arc<dyn StreamStore>) {
    let rss_kb = read_rss_kb();
    let fd_count = read_fd_count();
    let stream_count = streams.list().await.map(|s| s.len()).unwrap_or(0);

    info!(
        target: "oxplow::diagnostics",
        rss_kb = rss_kb.map(|n| n as i64).unwrap_or(-1),
        open_fds = fd_count.map(|n| n as i64).unwrap_or(-1),
        streams = stream_count,
        "self-diagnostics sample"
    );
}

/// Resident-set size in KB. Shells out to `ps`; works on macOS and
/// Linux without a new crate dep.
fn read_rss_kb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    std::str::from_utf8(&out.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Open file-descriptor count for the current process.
///
/// `/dev/fd` is present on both macOS and Linux and lists the calling
/// process's fds. We subtract one for the directory handle `read_dir`
/// itself uses — close enough for trend-watching, which is all we
/// want.
fn read_fd_count() -> Option<usize> {
    let dir = Path::new("/dev/fd");
    match std::fs::read_dir(dir) {
        Ok(iter) => Some(iter.count().saturating_sub(1)),
        Err(e) => {
            warn!(target: "oxplow::diagnostics", error = %e, "could not read /dev/fd");
            None
        }
    }
}
