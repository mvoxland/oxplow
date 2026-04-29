//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Tools are thin handlers that
//! delegate into `oxplow-app` services — we never duplicate business
//! logic between the Tauri command surface and the MCP tool surface.
//!
//! This commit ships a small starter set of tools to validate the
//! wiring (ping, app_version, list_streams, list_backlog). The full
//! 30+ tool surface from `src/mcp/**` ports incrementally — each new
//! tool is an `#[tool]` method on the `OxplowMcp` server type plus a
//! delegation into `oxplow-app`.
//!
//! Transport: stdio (the standard MCP wire format), so Claude Code
//! and other MCP clients can attach via subprocess.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};

use oxplow_app::Services;

#[derive(Clone)]
pub struct OxplowMcp {
    services: Arc<Services>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    /// Liveness check — used by Claude Code to verify the server is
    /// reachable. Mirrors the existing `mcp__oxplow__ping` tool.
    #[tool(description = "Liveness check: returns \"pong\".")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    /// Returns the running daemon version.
    #[tool(description = "Get the running oxplow daemon version.")]
    async fn app_version(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            env!("CARGO_PKG_VERSION"),
        )]))
    }

    /// Returns all streams in this project (primary + worktrees).
    #[tool(description = "List all streams in this project.")]
    async fn list_streams(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .streams
            .list_streams()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&list)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Returns all work items on the project-wide backlog.
    #[tool(description = "List all work items on the project backlog.")]
    async fn list_backlog(&self) -> Result<CallToolResult, McpError> {
        use oxplow_domain::stores::WorkItemStore;
        let list = self
            .services
            .work_item_store
            .list_backlog()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&list)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for OxplowMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Oxplow MCP server. Exposes the work-item, note, and stream surfaces. \
                 Authoritative tool list lives at .context/agent-model.md."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Convenience wrapper: spawn the server on stdio. The desktop
/// shell calls this from a Tauri sidecar/command when a client
/// connects.
pub async fn serve_stdio(services: Arc<Services>) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;
    let server = OxplowMcp::new(services);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_constructs() {
        let project = tempfile::tempdir().unwrap();
        let services = Arc::new(Services::in_memory(project.path()).unwrap());
        let _server = OxplowMcp::new(services);
    }

    #[tokio::test]
    async fn get_info_advertises_tool_capability() {
        let project = tempfile::tempdir().unwrap();
        let services = Arc::new(Services::in_memory(project.path()).unwrap());
        let server = OxplowMcp::new(services);
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }
}
