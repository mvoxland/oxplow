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

use oxplow_app::{CreateWorkItemInput, Services, UpdateWorkItemChanges};
use oxplow_domain::stores::{
    AgentStatusStore, HookEventStore, ThreadStore, WorkItemEventStore, WorkItemLinkStore,
    WorkItemStore, WorkNoteStore,
};
use oxplow_domain::{
    NoteId, ThreadId, WorkItem, WorkItemActorKind, WorkItemId, WorkItemKind, WorkItemLinkType,
    WorkItemStatus,
};

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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateWorkItemMcpParams {
    /// Optional thread to attach the new item to. Omit to create on
    /// the project-wide backlog.
    pub thread_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub kind: Option<String>,
    pub priority: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateWorkItemMcpParams {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CompleteTaskParams {
    pub id: String,
    /// Summary note appended to the work item before marking done.
    pub summary: String,
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LinkWorkItemsParams {
    pub thread_id: String,
    pub from_id: String,
    pub to_id: String,
    /// One of: blocks, relates_to, discovered_from, duplicates,
    /// supersedes, replies_to.
    pub link_type: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TransitionWorkItemsParams {
    pub ids: Vec<String>,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AwaitUserParams {
    pub thread_id: String,
    pub question: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetThreadContextParams {
    pub thread_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FileEpicWithChildrenParams {
    pub thread_id: Option<String>,
    pub epic_title: String,
    pub epic_description: Option<String>,
    pub children: Vec<EpicChildSpec>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EpicChildSpec {
    pub title: String,
    pub description: Option<String>,
    pub kind: Option<String>,
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

    // ---------- work item orchestration ----------

    #[tool(description = "Create a new work item. Allocates id + sort_index, fires creation event.")]
    async fn create_work_item(
        &self,
        params: Parameters<CreateWorkItemMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let thread = p.thread_id.map(ThreadId::from);
        let kind = match p.kind.as_deref() {
            Some(k) => Some(parse_kind(k)?),
            None => None,
        };
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        let item = self
            .services
            .work_items
            .create(
                thread,
                CreateWorkItemInput {
                    kind,
                    title: p.title,
                    description: p.description,
                    acceptance_criteria: None,
                    parent_id: p.parent_id.map(WorkItemId::from),
                    status: None,
                    priority,
                    category: p.category,
                    tags: p.tags,
                    author: Some(oxplow_domain::WorkItemAuthor::Agent),
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        json_result(&item)
    }

    #[tool(description = "Update fields on an existing work item (partial-patch).")]
    async fn update_work_item(
        &self,
        params: Parameters<UpdateWorkItemMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = WorkItemId::from(p.id);
        let status = match p.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
            None => None,
        };
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        let updated = self
            .services
            .work_items
            .update(
                &id,
                UpdateWorkItemChanges {
                    title: p.title,
                    description: p.description,
                    acceptance_criteria: None,
                    parent_id: None,
                    status,
                    priority,
                    category: None,
                    tags: None,
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        json_result(&updated)
    }

    #[tool(description = "Append a summary note to a work item then mark it `done`.")]
    async fn complete_task(
        &self,
        params: Parameters<CompleteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = WorkItemId::from(p.id);
        let author = p.author.unwrap_or_else(|| "agent".to_string());
        self.services
            .work_note_store
            .add_for_item(&id, &p.summary, &author)
            .await
            .map_err(internal)?;
        let item = self
            .services
            .work_items
            .update(
                &id,
                UpdateWorkItemChanges {
                    status: Some(WorkItemStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        json_result(&item)
    }

    #[tool(description = "Create a typed link between two work items.")]
    async fn link_work_items(
        &self,
        params: Parameters<LinkWorkItemsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let link_type = parse_link_type(&p.link_type)?;
        let link = self
            .services
            .work_item_link_store
            .create(
                &ThreadId::from(p.thread_id),
                &WorkItemId::from(p.from_id),
                &WorkItemId::from(p.to_id),
                link_type,
            )
            .await
            .map_err(internal)?;
        json_result(&link)
    }

    #[tool(description = "Transition a batch of work items to the same status.")]
    async fn transition_work_items(
        &self,
        params: Parameters<TransitionWorkItemsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let target = parse_status(&p.status)?;
        let mut updated = Vec::with_capacity(p.ids.len());
        for id in p.ids {
            let id = WorkItemId::from(id);
            let row = self
                .services
                .work_items
                .update(
                    &id,
                    UpdateWorkItemChanges {
                        status: Some(target),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            updated.push(row);
        }
        json_result(&updated)
    }

    #[tool(description = "Signal that the agent is awaiting user input. Persists a hook event so Stop suppression kicks in.")]
    async fn await_user(
        &self,
        params: Parameters<AwaitUserParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let payload = serde_json::json!({
            "await_user": true,
            "question": p.question,
        })
        .to_string();
        let event = oxplow_domain::HookEvent {
            id: oxplow_domain::HookEventId::new(),
            thread_id: Some(ThreadId::from(p.thread_id.clone())),
            stream_id: None,
            kind: oxplow_domain::HookKind::Stop,
            session_id: None,
            payload_json: payload,
            received_at: oxplow_domain::Timestamp::now(),
        };
        self.services
            .hook_event_store
            .append(&event)
            .await
            .map_err(internal)?;
        // Flip the agent_status to AwaitingUser directly so the
        // renderer reflects the state without needing a Stop hook.
        self.services
            .agent_status_store
            .upsert(
                &ThreadId::from(p.thread_id),
                "working",
                oxplow_domain::AgentStatusState::AwaitingUser,
                Some("await_user".into()),
            )
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("awaiting")]))
    }

    #[tool(description = "Bundle of thread state, work items, and recent activity.")]
    async fn get_thread_context(
        &self,
        params: Parameters<GetThreadContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = ThreadId::from(params.0.thread_id);
        let thread = self.services.thread_store.get(&id).await.map_err(internal)?;
        let items = self
            .services
            .work_item_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        let events = self
            .services
            .work_item_event_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        let bundle = serde_json::json!({
            "thread": thread,
            "items": items,
            "events": events,
        });
        Ok(CallToolResult::success(vec![Content::text(
            bundle.to_string(),
        )]))
    }

    #[tool(description = "Atomic: create an epic plus a list of children attached to it.")]
    async fn file_epic_with_children(
        &self,
        params: Parameters<FileEpicWithChildrenParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let thread = p.thread_id.map(ThreadId::from);
        let epic = self
            .services
            .work_items
            .create(
                thread.clone(),
                CreateWorkItemInput {
                    kind: Some(WorkItemKind::Epic),
                    title: p.epic_title,
                    description: p.epic_description,
                    author: Some(oxplow_domain::WorkItemAuthor::Agent),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        let mut children_out = Vec::with_capacity(p.children.len());
        for child in p.children {
            let kind = match child.kind.as_deref() {
                Some(k) => Some(parse_kind(k)?),
                None => Some(WorkItemKind::Task),
            };
            let row = self
                .services
                .work_items
                .create(
                    thread.clone(),
                    CreateWorkItemInput {
                        kind,
                        title: child.title,
                        description: child.description,
                        parent_id: Some(epic.id.clone()),
                        author: Some(oxplow_domain::WorkItemAuthor::Agent),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            children_out.push(row);
        }
        let bundle = serde_json::json!({ "epic": epic, "children": children_out });
        Ok(CallToolResult::success(vec![Content::text(
            bundle.to_string(),
        )]))
    }
}

fn parse_kind(s: &str) -> Result<WorkItemKind, McpError> {
    Ok(match s {
        "epic" => WorkItemKind::Epic,
        "task" => WorkItemKind::Task,
        "subtask" => WorkItemKind::Subtask,
        "bug" => WorkItemKind::Bug,
        "note" => WorkItemKind::Note,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown work item kind: {other}"),
                None,
            ))
        }
    })
}

fn parse_status(s: &str) -> Result<WorkItemStatus, McpError> {
    Ok(match s {
        "ready" => WorkItemStatus::Ready,
        "in_progress" => WorkItemStatus::InProgress,
        "blocked" => WorkItemStatus::Blocked,
        "done" => WorkItemStatus::Done,
        "canceled" => WorkItemStatus::Canceled,
        "archived" => WorkItemStatus::Archived,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown work item status: {other}"),
                None,
            ))
        }
    })
}

fn parse_priority(s: &str) -> Result<oxplow_domain::WorkItemPriority, McpError> {
    use oxplow_domain::WorkItemPriority as P;
    Ok(match s {
        "low" => P::Low,
        "medium" => P::Medium,
        "high" => P::High,
        "urgent" => P::Urgent,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown priority: {other}"),
                None,
            ))
        }
    })
}

fn parse_link_type(s: &str) -> Result<WorkItemLinkType, McpError> {
    Ok(match s {
        "blocks" => WorkItemLinkType::Blocks,
        "relates_to" => WorkItemLinkType::RelatesTo,
        "discovered_from" => WorkItemLinkType::DiscoveredFrom,
        "duplicates" => WorkItemLinkType::Duplicates,
        "supersedes" => WorkItemLinkType::Supersedes,
        "replies_to" => WorkItemLinkType::RepliesTo,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown link type: {other}"),
                None,
            ))
        }
    })
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
