//! MCP server for oxplow.
//!
//! Built on the official `rmcp` SDK. Tools are thin handlers that
//! delegate into `oxplow-app` services — we never duplicate business
//! logic between the Tauri command surface and the MCP tool surface.
//!
//! Each tool takes a single `Parameters<T>` argument (rmcp
//! convention); request shapes are defined as `serde + JsonSchema`
//! structs alongside the tool methods.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use oxplow_app::Services;
use oxplow_domain::stores::{ThreadStore, WorkItemStore, WorkNoteStore};
use oxplow_domain::{NoteId, ThreadId, WorkItem, WorkItemId};

#[derive(Clone)]
pub struct OxplowMcp {
    services: Arc<Services>,
    tool_router: ToolRouter<Self>,
}

// ---------- request shapes ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StreamIdParams {
    pub stream_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ThreadIdParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkItemIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpsertWorkItemParams {
    /// JSON-encoded WorkItem. Use this rather than nesting the struct
    /// directly so we don't have to plumb JsonSchema through every
    /// domain type.
    pub item_json: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddWorkNoteParams {
    pub work_item_id: String,
    pub body: String,
    pub author: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddThreadNoteParams {
    pub thread_id: String,
    pub body: String,
    pub author: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeleteNoteParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchParams {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SlugParams {
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddFollowupParams {
    pub thread_id: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FollowupIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SubsystemDocParams {
    pub name: String,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    // ---------- liveness / version ----------

    #[tool(description = "Liveness check: returns \"pong\".")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    #[tool(description = "Get the running oxplow daemon version.")]
    async fn app_version(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            env!("CARGO_PKG_VERSION"),
        )]))
    }

    // ---------- streams ----------

    #[tool(description = "List all streams (primary + worktrees) in this project.")]
    async fn list_streams(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .streams
            .list_streams()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    // ---------- threads ----------

    #[tool(description = "List threads attached to the given stream.")]
    async fn list_thread_work(
        &self,
        params: Parameters<StreamIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let stream_id = oxplow_domain::StreamId::from(params.0.stream_id);
        let list = self
            .services
            .thread_store
            .list_for_stream(&stream_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    // ---------- work items ----------

    #[tool(description = "List all work items on the project-wide backlog.")]
    async fn list_backlog(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .work_item_store
            .list_backlog()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(description = "List work items on a thread.")]
    async fn list_ready_work(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let thread_id = ThreadId::from(params.0.thread_id);
        let list = self
            .services
            .work_item_store
            .list_for_thread(&thread_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(description = "Get a single work item by id.")]
    async fn get_work_item(
        &self,
        params: Parameters<WorkItemIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = WorkItemId::from(params.0.id);
        let item = self
            .services
            .work_item_store
            .get(&id)
            .await
            .map_err(internal)?;
        json_result(&item)
    }

    #[tool(description = "Persist (insert or update) a work item. `item_json` is the JSON-encoded WorkItem.")]
    async fn upsert_work_item(
        &self,
        params: Parameters<UpsertWorkItemParams>,
    ) -> Result<CallToolResult, McpError> {
        let item: WorkItem = serde_json::from_str(&params.0.item_json)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        self.services
            .work_item_store
            .upsert(&item)
            .await
            .map_err(internal)?;
        json_result(&item)
    }

    #[tool(description = "Soft-delete a work item by id.")]
    async fn delete_work_item(
        &self,
        params: Parameters<WorkItemIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = WorkItemId::from(params.0.id);
        self.services
            .work_item_store
            .soft_delete(&id)
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- work notes ----------

    #[tool(description = "Add a work note attached to a specific work item.")]
    async fn add_work_note(
        &self,
        params: Parameters<AddWorkNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = WorkItemId::from(params.0.work_item_id);
        let note = self
            .services
            .work_note_store
            .add_for_item(&id, &params.0.body, &params.0.author)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

    #[tool(description = "Add a thread-scoped note (not attached to any item).")]
    async fn add_thread_note(
        &self,
        params: Parameters<AddThreadNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = ThreadId::from(params.0.thread_id);
        let note = self
            .services
            .work_note_store
            .add_for_thread(&id, &params.0.body, &params.0.author)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

    #[tool(description = "List notes for a work item.")]
    async fn list_work_notes(
        &self,
        params: Parameters<WorkItemIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = WorkItemId::from(params.0.id);
        let notes = self
            .services
            .work_note_store
            .list_for_item(&id)
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(description = "List thread-scoped notes.")]
    async fn list_thread_notes(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = ThreadId::from(params.0.thread_id);
        let notes = self
            .services
            .work_note_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(description = "Delete a note by id.")]
    async fn delete_note(
        &self,
        params: Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = NoteId::from(params.0.id);
        self.services
            .work_note_store
            .delete(&id)
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- wiki notes ----------

    #[tool(description = "List all wiki notes (metadata only).")]
    async fn list_notes(&self) -> Result<CallToolResult, McpError> {
        let notes = self
            .services
            .wiki_note_store
            .list()
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(description = "Title/slug glob search over wiki notes.")]
    async fn search_notes(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_note_store
            .search_titles(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "FTS5-backed body search over wiki notes; returns ranked snippets.")]
    async fn search_note_bodies(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_note_store
            .search_bodies(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "Get a wiki note's metadata by slug.")]
    async fn get_note_metadata(
        &self,
        params: Parameters<SlugParams>,
    ) -> Result<CallToolResult, McpError> {
        let note = self
            .services
            .wiki_note_store
            .get(&params.0.slug)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

    // ---------- followups ----------

    #[tool(description = "Add a followup reminder for a thread.")]
    async fn add_followup(
        &self,
        params: Parameters<AddFollowupParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = ThreadId::from(params.0.thread_id);
        let item = self.services.followups.add(id, params.0.body);
        json_result(&item)
    }

    #[tool(description = "List followups attached to a thread.")]
    async fn list_followups(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = ThreadId::from(params.0.thread_id);
        let list = self.services.followups.list_for_thread(&id);
        json_result(&list)
    }

    #[tool(description = "Remove a single followup by id.")]
    async fn remove_followup(
        &self,
        params: Parameters<FollowupIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.services.followups.remove(&params.0.id);
        Ok(CallToolResult::success(vec![Content::text("removed")]))
    }

    // ---------- subsystem docs ----------

    #[tool(description = "Read a `.context/<name>.md` subsystem doc; returns body + exists flag.")]
    async fn get_subsystem_doc(
        &self,
        params: Parameters<SubsystemDocParams>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .services
            .layout
            .project_dir
            .join(".context")
            .join(format!("{}.md", params.0.name));
        let exists = path.exists();
        let content = if exists {
            std::fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };
        let body = serde_json::json!({ "exists": exists, "content": content });
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for OxplowMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Oxplow MCP server. Exposes work-item, note, wiki, and stream surfaces. \
                 Authoritative tool list lives at .context/agent-model.md."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

fn internal<E: std::fmt::Display>(e: E) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

/// Convenience wrapper: spawn the server on stdio.
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
