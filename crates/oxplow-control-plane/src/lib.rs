//! In-process axum server that hosts the two surfaces the Claude Code
//! plugin needs to reach: hook delivery and the MCP protocol.
//!
//! Single TCP listener bound to `127.0.0.1:0` (ephemeral port). Two
//! routers:
//!
//! - `POST /hook/:event` — receives hook envelopes from the plugin's
//!   HTTP hooks, drains into [`oxplow_app::HookIngestService`].
//!   Bearer-auth via `Authorization: Bearer <hook_token>`.
//! - `POST /mcp` (and friends) — the rmcp Streamable HTTP transport
//!   wrapping [`oxplow_mcp::OxplowMcp`]. Same bearer token.
//!
//! Started once at boot from the Tauri main; the resulting
//! [`ControlPlane`] handle exposes `hook_base_url`, `mcp_endpoint_url`,
//! and `hook_token`, all of which the per-spawn plugin writer + agent-
//! command builder feed into env / config files.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{any_service, post},
    Json, Router,
};
use base64::Engine;
use rand::RngCore;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

use oxplow_app::{HookEnvelope, HookIngestService, Services};
use oxplow_domain::{HookKind, StreamId, ThreadId};

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Returned by [`spawn`]. The Tauri main keeps this alive for the life
/// of the process — dropping it does not stop the server (background
/// task is detached), but the URLs/token in it are what the plugin
/// writer needs.
#[derive(Debug, Clone)]
pub struct ControlPlane {
    pub bind_addr: SocketAddr,
    pub hook_token: String,
}

impl ControlPlane {
    /// Absolute URL the plugin's HTTP hooks POST to. Event name is
    /// appended as a path segment, e.g. `<base>/PreToolUse`.
    pub fn hook_base_url(&self) -> String {
        format!("http://{}/hook", self.bind_addr)
    }

    /// Absolute URL Claude Code uses for the MCP HTTP transport.
    pub fn mcp_endpoint_url(&self) -> String {
        format!("http://{}/mcp", self.bind_addr)
    }
}

#[derive(Clone)]
struct AppCtx {
    hook_ingest: HookIngestService,
    hook_token: Arc<String>,
}

/// Boot the control plane. Picks an ephemeral port on 127.0.0.1 and
/// returns immediately (the server runs in a detached tokio task).
pub async fn spawn(services: Arc<Services>) -> Result<ControlPlane, ControlPlaneError> {
    let token = generate_token();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = listener.local_addr()?;

    let ctx = AppCtx {
        hook_ingest: services.hook_ingest.clone(),
        hook_token: Arc::new(token.clone()),
    };

    let mcp_services = services.clone();
    let mcp_token = Arc::new(token.clone());

    // rmcp's StreamableHttpService is a tower::Service<Request>.
    // Mount it under /mcp via `any_service`. The factory closure runs
    // per-MCP-session to build a fresh OxplowMcp handler instance.
    let mcp_service = StreamableHttpService::new(
        move || Ok(oxplow_mcp::OxplowMcp::new(mcp_services.clone())),
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    // axum router for the MCP routes — wrap with our auth check.
    let mcp_auth_token = mcp_token.clone();
    let mcp_router = Router::new()
        .route_service("/mcp", any_service(mcp_service.clone()))
        .route_service("/mcp/", any_service(mcp_service))
        .layer(axum::middleware::from_fn(move |req, next| {
            let token = mcp_auth_token.clone();
            async move { auth_middleware(token, req, next).await }
        }));

    let hook_router = Router::new()
        .route("/hook/{event}", post(handle_hook))
        .with_state(ctx);

    let app = Router::new().merge(hook_router).merge(mcp_router);

    info!(addr = %bind_addr, "control plane listening");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app.into_make_service()).await {
            warn!(?err, "control plane server exited");
        }
    });

    Ok(ControlPlane {
        bind_addr,
        hook_token: token,
    })
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Bearer auth check. Constant-time comparison via base64 round-trip
/// avoidance — token strings are random base64 of equal length, so a
/// straight `==` is fine.
async fn auth_middleware(
    expected_token: Arc<String>,
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    if !check_bearer(req.headers(), &expected_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }
    next.run(req).await
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let Some(auth) = headers.get(http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = auth.to_str() else {
        return false;
    };
    let Some(rest) = s.strip_prefix("Bearer ") else {
        return false;
    };
    rest == expected
}

async fn handle_hook(
    State(ctx): State<AppCtx>,
    AxumPath(event): AxumPath<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_bearer(&headers, &ctx.hook_token) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
    }

    let kind = match parse_hook_kind(&event) {
        Some(k) => k,
        None => {
            // Unknown but non-fatal — record nothing, ack so the agent
            // doesn't block.
            return (StatusCode::ACCEPTED, "ignored unknown hook event").into_response();
        }
    };

    let stream_id = headers
        .get("x-oxplow-stream")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| StreamId::from(s.to_string()));
    let thread_id = headers
        .get("x-oxplow-thread")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| ThreadId::from(s.to_string()));

    let body_str = match std::str::from_utf8(&body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "non-utf8 body").into_response();
        }
    };

    let session_id = extract_session_id(&body_str);
    let prompt = extract_prompt(&body_str, kind);

    let envelope = HookEnvelope {
        kind,
        thread_id,
        stream_id,
        session_id,
        payload_json: body_str,
        prompt,
    };

    match ctx.hook_ingest.ingest(envelope).await {
        Ok(_) => (StatusCode::ACCEPTED, Json(serde_json::json!({}))).into_response(),
        Err(err) => {
            warn!(?event, ?err, "hook ingest failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("ingest failed: {err}"),
            )
                .into_response()
        }
    }
}

fn parse_hook_kind(event: &str) -> Option<HookKind> {
    match event {
        "PreToolUse" => Some(HookKind::PreToolUse),
        "PostToolUse" => Some(HookKind::PostToolUse),
        "UserPromptSubmit" => Some(HookKind::UserPromptSubmit),
        "Stop" => Some(HookKind::Stop),
        // SessionStart / SessionEnd / Notification aren't on the
        // HookKind enum yet — they're informational from oxplow's
        // perspective. Returning None routes them to ACCEPTED above
        // without persisting. AgentBoot, SubagentStop, Interrupt are
        // synthetic / not posted by the plugin.
        _ => None,
    }
}

/// Best-effort: pull `session_id` out of the envelope JSON. Claude
/// Code includes it on every hook payload; we're tolerant if a future
/// version moves the field.
fn extract_session_id(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("session_id")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// For UserPromptSubmit hooks, pull the visible prompt body so the
/// new agent_turn row can render it. Claude Code's payload puts the
/// prompt at `prompt` (top level); we fall back gracefully.
fn extract_prompt(body: &str, kind: HookKind) -> Option<String> {
    if kind != HookKind::UserPromptSubmit {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("prompt")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_long_enough() {
        let t = generate_token();
        // 32 bytes base64-url-no-pad → 43 chars.
        assert_eq!(t.len(), 43);
    }

    #[test]
    fn bearer_check_accepts_matching() {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer abc"),
        );
        assert!(check_bearer(&h, "abc"));
    }

    #[test]
    fn bearer_check_rejects_missing() {
        assert!(!check_bearer(&HeaderMap::new(), "abc"));
    }

    #[test]
    fn bearer_check_rejects_wrong() {
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer xyz"),
        );
        assert!(!check_bearer(&h, "abc"));
    }

    #[test]
    fn parse_hook_kind_known() {
        assert!(matches!(parse_hook_kind("PreToolUse"), Some(HookKind::PreToolUse)));
        assert!(matches!(parse_hook_kind("Stop"), Some(HookKind::Stop)));
    }

    #[test]
    fn parse_hook_kind_unknown_returns_none() {
        assert!(parse_hook_kind("SessionStart").is_none());
        assert!(parse_hook_kind("garbage").is_none());
    }

    #[test]
    fn extract_prompt_only_on_userpromptsubmit() {
        let body = r#"{"prompt":"hi","session_id":"s1"}"#;
        assert_eq!(
            extract_prompt(body, HookKind::UserPromptSubmit),
            Some("hi".into())
        );
        assert_eq!(extract_prompt(body, HookKind::Stop), None);
    }
}
