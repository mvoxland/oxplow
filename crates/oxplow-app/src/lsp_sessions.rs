//! LSP session manager.
//!
//! Caches one `LspProxy` per `(stream, language)` so JSON-RPC requests
//! issued by the renderer or MCP tools don't pay the spawn-and-init
//! cost on every call. Initialization is lazy: the first request for
//! a given pair spawns the language server and runs the LSP
//! `initialize` handshake.
//!
//! Session lookup uses the `LspServerConfig` from `oxplow.yaml` to
//! find the command for a given language; if no config is present
//! the call returns an error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

use oxplow_config::{LspServerConfig, OxplowConfig};
use oxplow_lsp::{LspError, LspProxy, SpawnConfig};

#[derive(Debug, Error)]
pub enum LspSessionError {
    #[error("no language server configured for language `{0}`")]
    NoConfig(String),
    #[error("lsp: {0}")]
    Lsp(#[from] LspError),
    #[error("lsp not initialized for language `{0}`")]
    NotInitialized(String),
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct SessionKey {
    stream_id: String,
    language: String,
}

#[derive(Clone)]
pub struct LspSessionManager {
    config: Arc<std::sync::RwLock<OxplowConfig>>,
    sessions: Arc<Mutex<HashMap<SessionKey, Arc<LspProxy>>>>,
}

impl LspSessionManager {
    pub fn new(config: Arc<std::sync::RwLock<OxplowConfig>>) -> Self {
        Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn find_server_config(&self, language: &str) -> Option<LspServerConfig> {
        let cfg = self.config.read().ok()?;
        cfg.lsp_servers
            .iter()
            .find(|s| s.language_id == language)
            .cloned()
    }

    /// Get or spawn the LspProxy for `(stream_id, language)`.
    pub async fn ensure(
        &self,
        stream_id: &str,
        language: &str,
        cwd: PathBuf,
    ) -> Result<Arc<LspProxy>, LspSessionError> {
        let key = SessionKey {
            stream_id: stream_id.to_string(),
            language: language.to_string(),
        };
        {
            let map = self.sessions.lock().await;
            if let Some(p) = map.get(&key) {
                return Ok(p.clone());
            }
        }
        let server_config = self
            .find_server_config(language)
            .ok_or_else(|| LspSessionError::NoConfig(language.to_string()))?;

        let proxy = LspProxy::spawn(SpawnConfig {
            command: server_config.command,
            args: server_config.args,
            cwd: Some(cwd.clone()),
        })?;

        // Run the LSP `initialize` handshake. We don't carry full
        // capabilities — minimum is enough to satisfy
        // typescript-language-server.
        let init = proxy
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": format!("file://{}", cwd.display()),
                    "capabilities": {},
                    "workspaceFolders": [{
                        "uri": format!("file://{}", cwd.display()),
                        "name": "oxplow",
                    }],
                }),
            )
            .await?;
        info!(?language, "lsp initialized");
        proxy.notify("initialized", json!({})).await?;
        let _ = init;

        let arc = Arc::new(proxy);
        let mut map = self.sessions.lock().await;
        map.insert(key, arc.clone());
        Ok(arc)
    }

    /// Tear down all sessions for a stream (e.g. on stream delete).
    /// Best-effort; we don't shutdown LSPs cleanly here, just drop.
    pub async fn drop_for_stream(&self, stream_id: &str) {
        let mut map = self.sessions.lock().await;
        let keys: Vec<_> = map
            .keys()
            .filter(|k| k.stream_id == stream_id)
            .cloned()
            .collect();
        for key in keys {
            if map.remove(&key).is_some() {
                warn!(stream_id, language = key.language, "lsp session dropped");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::AgentKind;

    fn empty_config() -> Arc<std::sync::RwLock<OxplowConfig>> {
        Arc::new(std::sync::RwLock::new(OxplowConfig {
            agent: AgentKind::Claude,
            project_name: "p".into(),
            lsp_servers: vec![],
            agent_prompt_append: String::new(),
            snapshot_retention_days: 7,
            generated_dirs: vec![],
            snapshot_max_file_bytes: 0,
            inject_session_context: true,
        }))
    }

    #[tokio::test]
    async fn ensure_returns_no_config_when_language_unknown() {
        let mgr = LspSessionManager::new(empty_config());
        let err = mgr
            .ensure("s-1", "tsx", std::env::temp_dir())
            .await
            .err()
            .expect("should error");
        assert!(matches!(err, LspSessionError::NoConfig(lang) if lang == "tsx"));
    }
}
