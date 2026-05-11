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

use oxplow_app::{CreateTaskInput, OxplowEvent, Services, UpdateTaskChanges};
use oxplow_domain::stores::{TaskEventStore, TaskLinkStore, TaskStore, ThreadStore, TaskNoteStore};
use oxplow_domain::{NoteId, Task, TaskId, TaskLinkType, TaskStatus, ThreadId};

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
pub struct TaskIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReorderTasksParams {
    /// Optional thread scope. Omit for the project-wide backlog.
    pub thread_id: Option<String>,
    /// New sort order. Items not present keep their relative order
    /// at the end of the list.
    pub ordered_item_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DelegateQueryParams {
    pub thread_id: String,
    pub question: String,
    pub focus: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RecordQueryFindingParams {
    pub note_id: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpsertTaskParams {
    /// JSON-encoded Task. Use this rather than nesting the struct
    /// directly so we don't have to plumb JsonSchema through every
    /// domain type.
    pub item_json: String,
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
pub struct CreateTaskMcpParams {
    /// Thread to attach the new item to. Required unless `backlog`
    /// is set to `true` — filing onto the project-wide backlog must
    /// be an explicit choice, since a thread-detached row trips
    /// filing-enforcement on the next edit.
    pub thread_id: Option<String>,
    /// Set to `true` to file the item onto the project-wide backlog
    /// (no thread attachment). Mutually exclusive with `thread_id`.
    /// Default `false`: a missing `thread_id` is an error.
    #[serde(default)]
    pub backlog: bool,
    pub title: String,
    pub description: Option<String>,
    /// One observable criterion per line. Authoritative completion
    /// signal; reviewers + complete_task scan for it.
    pub acceptance_criteria: Option<String>,
    pub kind: Option<String>,
    pub priority: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub parent_id: Option<String>,
    /// Initial status — defaults to `ready`. Pass `in_progress`
    /// when starting the work in the same call (filing-enforcement
    /// requires an in_progress row to exist before edits land), or
    /// `done`/`blocked` when filing a row for already-shipped work
    /// (`touched_files` then drives Local History attribution).
    pub status: Option<String>,
    /// Repo-relative paths edited for this effort. When passed
    /// alongside `status: "done"` or `"blocked"`, the runtime
    /// synthesizes the in_progress→target effort transition so
    /// Local History attributes the writes to this item.
    pub touched_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateTaskMcpParams {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Replace the AC list. Pass an empty string to clear.
    pub acceptance_criteria: Option<String>,
    /// Reparent (or detach with empty string).
    pub parent_id: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    /// Repo-relative paths edited for the effort that's closing
    /// alongside this update. Required for Local History attribution
    /// when transitioning to `done`/`blocked` from `in_progress`.
    pub touched_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CompleteTaskParams {
    pub id: String,
    /// Summary note appended to the task before marking done.
    pub summary: String,
    pub author: Option<String>,
    /// Repo-relative paths edited for this effort. Drives the file-
    /// attribution effort row Local History reads from.
    pub touched_files: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LinktasksParams {
    pub thread_id: String,
    pub from_id: String,
    pub to_id: String,
    /// One of: blocks, relates_to, discovered_from, duplicates,
    /// supersedes, replies_to.
    pub link_type: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TransitiontasksParams {
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DispatchTaskParams {
    pub thread_id: String,
    /// The specific task to dispatch. When omitted, picks the
    /// first ready item on the thread (mirrors main's
    /// dispatch-without-id shortcut for /work-next composition).
    pub item_id: Option<String>,
    /// Optional extra context appended to the brief — usually
    /// orchestrator notes about how this fits into the larger plan.
    pub extra_context: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ForkThreadParams {
    pub source_thread_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FindNotesForNoteParams {
    pub slug: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PageRefParams {
    /// Page kind, e.g. "wiki", "task", "file", "git-commit",
    /// "finding", "directory".
    pub kind: String,
    /// Canonical page id within the kind. For files this is the
    /// repo-relative path; for tasks the `wi-…` id; for
    /// commits the full sha; for wiki pages the slug.
    pub id: String,
    #[serde(default = "default_page_ref_limit")]
    pub limit: u32,
}

fn default_page_ref_limit() -> u32 {
    100
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ResyncNoteParams {
    pub slug: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LspPositionParams {
    pub stream_id: String,
    pub language: String,
    pub uri: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LspDiagnosticsParams {
    pub stream_id: String,
    pub language: String,
    pub uri: String,
}

#[tool_router]
impl OxplowMcp {
    pub fn new(services: Arc<Services>) -> Self {
        Self {
            services,
            tool_router: Self::tool_router(),
        }
    }

    /// Emit `TasksChanged` so the renderer (which is a separate
    /// process from the MCP server) refetches and reflects the
    /// mutation. The Tauri command layer emits its own events; MCP
    /// has to do the same or UI state silently goes stale after every
    /// agent-driven change.
    fn emit_tasks_changed(&self, thread_id: Option<oxplow_domain::ThreadId>) {
        self.services
            .events
            .emit(OxplowEvent::TasksChanged { thread_id });
    }

    // ---------- liveness / version ----------

    #[tool(description = "Liveness check: returns \"pong\".")]
    async fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text("pong")]))
    }

    #[tool(description = "Get the running oxplow daemon version.")]
    async fn app_version(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(env!(
            "CARGO_PKG_VERSION"
        ))]))
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
        expect_id_kind(
            "list_thread_work",
            "stream_id",
            &params.0.stream_id,
            ID_STREAM,
        )?;
        let stream_id = oxplow_domain::StreamId::from(params.0.stream_id);
        let list = self
            .services
            .thread_store
            .list_for_stream(&stream_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    // ---------- tasks ----------

    #[tool(description = "List all tasks on the project-wide backlog.")]
    async fn list_backlog(&self) -> Result<CallToolResult, McpError> {
        let list = self
            .services
            .task_store
            .list_backlog()
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(description = "List tasks on a thread.")]
    async fn list_ready_work(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_ready_work",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = ThreadId::from(params.0.thread_id);
        let list = self
            .services
            .task_store
            .list_for_thread(&thread_id)
            .await
            .map_err(internal)?;
        json_result(&list)
    }

    #[tool(
        description = "Return the next dispatch unit for the orchestrator. If the highest-priority \
                       ready item is an epic, returns the epic and all its ready descendants as one \
                       atomic unit. Otherwise returns all ready non-epic items so you can pick one or \
                       a related cluster to dispatch. Honors `blocks` links — items waiting on a \
                       non-done blocker are skipped. Returns { mode: \"empty\" } when nothing is ready."
    )]
    async fn read_task_options(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "read_task_options",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = ThreadId::from(params.0.thread_id);
        let result = self
            .services
            .tasks
            .read_task_options(&thread_id, &*self.services.task_link_store)
            .await
            .map_err(internal)?;
        json_result(&result)
    }

    #[tool(
        description = "Reorder tasks on a thread (or backlog). The orderedItemIds array becomes \
                       the new sort order; items not in the list keep their relative order at the end."
    )]
    async fn reorder_tasks(
        &self,
        params: Parameters<ReorderTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(t) = params.0.thread_id.as_deref() {
            expect_id_kind("reorder_tasks", "thread_id", t, ID_THREAD)?;
        }
        let mut ids: Vec<TaskId> = Vec::with_capacity(params.0.ordered_item_ids.len());
        for raw in &params.0.ordered_item_ids {
            ids.push(parse_task_id("reorder_tasks", "ordered_item_ids[]", raw)?);
        }
        let thread = params
            .0
            .thread_id
            .as_deref()
            .map(|s| ThreadId::from(s.to_string()));
        self.services
            .tasks
            .reorder(thread.as_ref(), &ids)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(thread);
        json_result(&serde_json::json!({ "ok": true }))
    }

    #[tool(description = "Get a single task by id.")]
    async fn get_task(&self, params: Parameters<TaskIdParams>) -> Result<CallToolResult, McpError> {
        let id = parse_task_id("get_task", "id", &params.0.id)?;
        let item = self.services.task_store.get(id).await.map_err(internal)?;
        json_result(&item)
    }

    #[tool(
        description = "Persist (insert or update) a task. `item_json` is the JSON-encoded Task."
    )]
    async fn upsert_task(
        &self,
        params: Parameters<UpsertTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut item: Task = serde_json::from_str(&params.0.item_json)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        if item.id.value() == 0 {
            let new_id = self
                .services
                .task_store
                .insert(&item)
                .await
                .map_err(internal)?;
            item.id = new_id;
        } else {
            self.services
                .task_store
                .update(&item)
                .await
                .map_err(internal)?;
        }
        self.emit_tasks_changed(item.thread_id.clone());
        json_result(&item)
    }

    #[tool(description = "Soft-delete a task by id.")]
    async fn delete_task(
        &self,
        params: Parameters<TaskIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_task_id("delete_task", "id", &params.0.id)?;
        let item = self.services.task_store.get(id).await.map_err(internal)?;
        self.services
            .task_store
            .soft_delete(id)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(item.and_then(|i| i.thread_id));
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- thread notes ----------
    //
    // Per-task notes (`add_work_note` / `list_work_notes`) were
    // retired: `task_effort.summary` already carries "what
    // shipped on this item", so a parallel note table for the same
    // purpose was duplicative. Thread-scoped notes stay — they back
    // the Explore-subagent findings flow.

    #[tool(description = "Add a thread-scoped note (not attached to any item).")]
    async fn add_thread_note(
        &self,
        params: Parameters<AddThreadNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "add_thread_note",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let note = self
            .services
            .work_note_store
            .add_for_thread(&id, &params.0.body, &params.0.author)
            .await
            .map_err(internal)?;
        json_result(&note)
    }

    #[tool(description = "List thread-scoped notes.")]
    async fn list_thread_notes(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_thread_notes",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let notes = self
            .services
            .work_note_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(
        description = "Prepare an exploration query for an Explore subagent. Use when you need to \
                       understand a codebase area before dispatching real work and would otherwise \
                       read 5+ files inline — offloading the reads keeps your own cached context \
                       small. Returns { prompt, provisionalNoteId }. The orchestrator then calls \
                       Agent(subagent_type='Explore', prompt=<prompt>); the prompt instructs the \
                       subagent to call mcp__oxplow__record_query_finding({ noteId: \
                       <provisionalNoteId>, body }) with its findings. Read the finding later via \
                       mcp__oxplow__list_thread_notes."
    )]
    async fn delegate_query(
        &self,
        params: Parameters<DelegateQueryParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "delegate_query",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let thread_id = ThreadId::from(params.0.thread_id.clone());
        let question = params.0.question.trim().to_string();
        if question.is_empty() {
            return Err(McpError::invalid_params(
                "delegate_query: `question` is required",
                None,
            ));
        }
        let focus = params.0.focus.unwrap_or_default().trim().to_string();
        // Allocate the finding note up front with an empty body. The
        // subagent fills it in via record_query_finding when done.
        let provisional = self
            .services
            .work_note_store
            .add_for_thread(&thread_id, "", "explore-subagent")
            .await
            .map_err(internal)?;
        let prompt = compose_delegate_query_prompt(
            &params.0.thread_id,
            &question,
            &focus,
            provisional.id.as_str(),
        );
        json_result(&serde_json::json!({
            "ok": true,
            "prompt": prompt,
            "provisionalNoteId": provisional.id.as_str(),
        }))
    }

    #[tool(
        description = "Write the Explore subagent's finding into a pre-allocated thread-scoped note \
                       (id returned by mcp__oxplow__delegate_query). Call this once at the end of \
                       the exploration — the orchestrator reads it later via list_thread_notes."
    )]
    async fn record_query_finding(
        &self,
        params: Parameters<RecordQueryFindingParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.0.note_id.is_empty() {
            return Err(McpError::invalid_params(
                "record_query_finding: `noteId` is required",
                None,
            ));
        }
        expect_id_kind(
            "record_query_finding",
            "note_id",
            &params.0.note_id,
            ID_NOTE,
        )?;
        let id = NoteId::from(params.0.note_id.clone());
        self.services
            .work_note_store
            .update_body(&id, &params.0.body)
            .await
            .map_err(internal)?;
        json_result(&serde_json::json!({ "ok": true, "noteId": params.0.note_id }))
    }

    #[tool(description = "Delete a note by id.")]
    async fn delete_wiki_page(
        &self,
        params: Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("delete_wiki_page", "id", &params.0.id, ID_NOTE)?;
        let id = NoteId::from(params.0.id);
        self.services
            .work_note_store
            .delete(&id)
            .await
            .map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text("deleted")]))
    }

    // ---------- wiki pages ----------

    #[tool(description = "List all wiki pages (metadata only).")]
    async fn list_wiki_pages(&self) -> Result<CallToolResult, McpError> {
        let notes = self
            .services
            .wiki_page_store
            .list()
            .await
            .map_err(internal)?;
        json_result(&notes)
    }

    #[tool(description = "Title/slug glob search over wiki pages.")]
    async fn search_wiki_pages(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_page_store
            .search_titles(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "FTS5-backed body search over wiki pages; returns ranked snippets.")]
    async fn search_wiki_page_bodies(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let hits = self
            .services
            .wiki_page_store
            .search_bodies(&params.0.query, params.0.limit as usize)
            .await
            .map_err(internal)?;
        json_result(&hits)
    }

    #[tool(description = "Get a wiki page's metadata by slug.")]
    async fn get_wiki_page_metadata(
        &self,
        params: Parameters<SlugParams>,
    ) -> Result<CallToolResult, McpError> {
        let note = self
            .services
            .wiki_page_store
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
        expect_id_kind("add_followup", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let id = ThreadId::from(params.0.thread_id);
        let item = self.services.followups.add(id, params.0.body);
        json_result(&item)
    }

    #[tool(description = "List followups attached to a thread.")]
    async fn list_followups(
        &self,
        params: Parameters<ThreadIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "list_followups",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let list = self.services.followups.list_for_thread(&id);
        json_result(&list)
    }

    #[tool(description = "Remove a single followup by id.")]
    async fn remove_followup(
        &self,
        params: Parameters<FollowupIdParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("remove_followup", "id", &params.0.id, ID_FOLLOWUP)?;
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

    // ---------- task orchestration ----------

    #[tool(
        description = "Create a new task. Allocates id + sort_index, fires creation event. \
                       `thread_id` is required unless `backlog: true` is set (a thread-detached \
                       row trips filing-enforcement on the next edit, so backlog filing must be \
                       an explicit choice). Pass `status: \"in_progress\"` to start the work in \
                       the same call (filing-enforcement requires an in_progress row to exist \
                       before edits land). Pass `status: \"done\"` (or `blocked`) with \
                       `touched_files` to file a row for already-shipped work — the runtime \
                       synthesizes the in_progress→target effort so Local History attributes \
                       the writes."
    )]
    async fn create_task(
        &self,
        params: Parameters<CreateTaskMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match (p.thread_id.as_deref(), p.backlog) {
            (Some(_), true) => {
                return Err(McpError::invalid_params(
                    "create_task: pass `thread_id` OR `backlog: true`, not both",
                    None,
                ));
            }
            (None, false) => {
                return Err(McpError::invalid_params(
                    "create_task: `thread_id` is required (or set `backlog: true` to file \
                     onto the project-wide backlog)",
                    None,
                ));
            }
            _ => {}
        }
        if let Some(tid) = p.thread_id.as_deref() {
            expect_id_kind("create_task", "thread_id", tid, ID_THREAD)?;
        }
        let parent_task_id = match p.parent_id.as_deref() {
            Some(pid) => Some(parse_task_id("create_task", "parent_id", pid)?),
            None => None,
        };
        let thread = p.thread_id.clone().map(ThreadId::from);
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        let status = match p.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
            None => None,
        };
        let item = self
            .services
            .tasks
            .create(
                thread.clone(),
                CreateTaskInput {
                    title: p.title,
                    description: p.description,
                    acceptance_criteria: p.acceptance_criteria,
                    parent_id: parent_task_id,
                    status,
                    priority,
                    category: p.category,
                    tags: p.tags,
                    author: Some(oxplow_domain::TaskAuthor::Agent),
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        // Synthesize the in_progress→target effort when the row was
        // filed directly into a closing state with touched files.
        // Mirrors main: a `done`/`blocked` create with `touchedFiles`
        // is the "file and close in one call" shortcut for retroactive
        // splits, and Local History needs the effort row to attribute
        // the writes to this item.
        let touched = p.touched_files.unwrap_or_default();
        if !touched.is_empty() && matches!(item.status, TaskStatus::Done | TaskStatus::Blocked) {
            let thread_for_effort = thread.or_else(|| item.thread_id.clone());
            if let Some(tid) = thread_for_effort {
                if let Err(err) = self
                    .services
                    .tasks
                    .record_effort(&self.services.effort_store, item.id, &tid, &touched, None)
                    .await
                {
                    tracing::warn!(?err, "create_task: effort record failed");
                }
            }
        }
        self.emit_tasks_changed(item.thread_id.clone());
        json_result(&item)
    }

    #[tool(
        description = "Update fields on an existing task (partial-patch). Pass `touched_files` \
                       alongside a `status` transition to `done`/`blocked` to attribute the closing \
                       effort. Pass `acceptance_criteria` (empty string clears) to update the AC list. \
                       `parent_id` reparents (empty string detaches)."
    )]
    async fn update_task(
        &self,
        params: Parameters<UpdateTaskMcpParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = parse_task_id("update_task", "id", &p.id)?;
        if let Some(pid) = p.parent_id.as_deref() {
            // Empty string is the "detach" sentinel — only validate non-empty.
            if !pid.is_empty() {
                parse_task_id("update_task", "parent_id", pid)?;
            }
        }
        let status = match p.status.as_deref() {
            Some(s) => Some(parse_status(s)?),
            None => None,
        };
        let priority = match p.priority.as_deref() {
            Some(s) => Some(parse_priority(s)?),
            None => None,
        };
        // Acceptance-criteria + parent: `Option<Option<…>>` semantics
        // — outer Some means "the field was passed", inner None means
        // "clear it". Empty string = clear; non-empty = set.
        let acceptance_criteria: Option<Option<String>> =
            p.acceptance_criteria
                .map(|s| if s.is_empty() { None } else { Some(s) });
        let parent_id: Option<Option<TaskId>> = match p.parent_id {
            Some(s) if s.is_empty() => Some(None),
            Some(s) => Some(Some(parse_task_id("update_task", "parent_id", &s)?)),
            None => None,
        };
        let updated = self
            .services
            .tasks
            .update(
                id,
                UpdateTaskChanges {
                    title: p.title,
                    description: p.description,
                    acceptance_criteria,
                    parent_id,
                    status,
                    priority,
                    category: None,
                    tags: None,
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let touched = p.touched_files.unwrap_or_default();
        if !touched.is_empty() && matches!(updated.status, TaskStatus::Done | TaskStatus::Blocked) {
            if let Some(tid) = updated.thread_id.clone() {
                if let Err(err) = self
                    .services
                    .tasks
                    .record_effort(
                        &self.services.effort_store,
                        updated.id,
                        &tid,
                        &touched,
                        None,
                    )
                    .await
                {
                    tracing::warn!(?err, "update_task: effort record failed");
                }
            }
        }
        self.emit_tasks_changed(updated.thread_id.clone());
        json_result(&updated)
    }

    #[tool(
        description = "Append a summary note to a task then mark it `done`. Pass \
                       `touched_files` (repo-relative paths edited for this effort) to attribute \
                       the writes via Local History — skip only if you edited >100 files."
    )]
    async fn complete_task(
        &self,
        params: Parameters<CompleteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = parse_task_id("complete_task", "id", &p.id)?;
        let author = p.author.unwrap_or_else(|| "agent".to_string());
        self.services
            .work_note_store
            .add_for_item(id, &p.summary, &author)
            .await
            .map_err(internal)?;
        let item = self
            .services
            .tasks
            .update(
                id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let touched = p.touched_files.unwrap_or_default();
        if !touched.is_empty() {
            if let Some(tid) = item.thread_id.clone() {
                if let Err(err) = self
                    .services
                    .tasks
                    .record_effort(
                        &self.services.effort_store,
                        item.id,
                        &tid,
                        &touched,
                        Some(p.summary.clone()),
                    )
                    .await
                {
                    tracing::warn!(?err, "complete_task: effort record failed");
                }
            }
        }
        self.emit_tasks_changed(item.thread_id.clone());
        json_result(&item)
    }

    #[tool(description = "Create a typed link between two tasks.")]
    async fn link_tasks(
        &self,
        params: Parameters<LinktasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("link_tasks", "thread_id", &p.thread_id, ID_THREAD)?;
        let from_id = parse_task_id("link_tasks", "from_id", &p.from_id)?;
        let to_id = parse_task_id("link_tasks", "to_id", &p.to_id)?;
        let link_type = parse_link_type(&p.link_type)?;
        let thread = ThreadId::from(p.thread_id);
        let link = self
            .services
            .task_link_store
            .create(&thread, from_id, to_id, link_type)
            .await
            .map_err(internal)?;
        self.emit_tasks_changed(Some(thread));
        json_result(&link)
    }

    #[tool(description = "Transition a batch of tasks to the same status.")]
    async fn transition_tasks(
        &self,
        params: Parameters<TransitiontasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mut parsed_ids: Vec<TaskId> = Vec::with_capacity(p.ids.len());
        for raw in &p.ids {
            parsed_ids.push(parse_task_id("transition_tasks", "ids[]", raw)?);
        }
        let target = parse_status(&p.status)?;
        let mut updated = Vec::with_capacity(parsed_ids.len());
        for id in parsed_ids {
            let row = self
                .services
                .tasks
                .update(
                    id,
                    UpdateTaskChanges {
                        status: Some(target),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            updated.push(row);
        }
        let mut threads: std::collections::HashSet<Option<oxplow_domain::ThreadId>> =
            std::collections::HashSet::new();
        for row in &updated {
            threads.insert(row.thread_id.clone());
        }
        for tid in threads {
            self.emit_tasks_changed(tid);
        }
        json_result(&updated)
    }

    #[tool(
        description = "Signal that the agent is awaiting user input. Persists a hook event so Stop suppression kicks in."
    )]
    async fn await_user(
        &self,
        params: Parameters<AwaitUserParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        expect_id_kind("await_user", "thread_id", &p.thread_id, ID_THREAD)?;
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

    #[tool(description = "Bundle of thread state, tasks, and recent activity.")]
    async fn get_thread_context(
        &self,
        params: Parameters<GetThreadContextParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "get_thread_context",
            "thread_id",
            &params.0.thread_id,
            ID_THREAD,
        )?;
        let id = ThreadId::from(params.0.thread_id);
        let thread = self
            .services
            .thread_store
            .get(&id)
            .await
            .map_err(internal)?;
        let items = self
            .services
            .task_store
            .list_for_thread(&id)
            .await
            .map_err(internal)?;
        let events = self
            .services
            .task_event_store
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
        if let Some(t) = p.thread_id.as_deref() {
            expect_id_kind("file_epic_with_children", "thread_id", t, ID_THREAD)?;
        }
        let thread = p.thread_id.map(ThreadId::from);
        let epic = self
            .services
            .tasks
            .create(
                thread.clone(),
                CreateTaskInput {
                    title: p.epic_title,
                    description: p.epic_description,
                    author: Some(oxplow_domain::TaskAuthor::Agent),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        let mut children_out = Vec::with_capacity(p.children.len());
        for child in p.children {
            let row = self
                .services
                .tasks
                .create(
                    thread.clone(),
                    CreateTaskInput {
                        title: child.title,
                        description: child.description,
                        parent_id: Some(epic.id),
                        author: Some(oxplow_domain::TaskAuthor::Agent),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| internal(e.to_string()))?;
            children_out.push(row);
        }
        self.emit_tasks_changed(thread.clone());
        let bundle = serde_json::json!({ "epic": epic, "children": children_out });
        Ok(CallToolResult::success(vec![Content::text(
            bundle.to_string(),
        )]))
    }

    #[tool(
        description = "Compose a ready-to-paste dispatch brief for a task and transition it \
                       to in_progress in one atomic call. When `item_id` is given, dispatches that \
                       specific item; otherwise picks the first ready non-epic item on the thread \
                       (mirrors main's /work-next composition shortcut). Returns \
                       `{ ok, prompt, itemId }` — pass `prompt` to the general-purpose Agent tool. \
                       The brief carries the item fields, AC, recent notes, and the subagent \
                       protocol preamble so the orchestrator brief stays slim."
    )]
    async fn dispatch_task(
        &self,
        params: Parameters<DispatchTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind("dispatch_task", "thread_id", &params.0.thread_id, ID_THREAD)?;
        let parsed_item_id = match params.0.item_id.as_deref() {
            Some(raw) => Some(parse_task_id("dispatch_task", "item_id", raw)?),
            None => None,
        };
        let thread_id = ThreadId::from(params.0.thread_id.clone());
        let target = match parsed_item_id {
            Some(id) => self
                .services
                .task_store
                .get(id)
                .await
                .map_err(internal)?
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!("dispatch_task: item not found: {}", id.value()),
                        None,
                    )
                })?,
            None => {
                let items = self
                    .services
                    .task_store
                    .list_for_thread(&thread_id)
                    .await
                    .map_err(internal)?;
                // Build a set of task ids that have children → epics.
                let epic_ids: std::collections::HashSet<TaskId> =
                    items.iter().filter_map(|i| i.parent_id).collect();
                let mut ready_first: Vec<_> = items
                    .into_iter()
                    .filter(|i| {
                        matches!(i.status, oxplow_domain::TaskStatus::Ready)
                            && !epic_ids.contains(&i.id)
                    })
                    .collect();
                ready_first.sort_by_key(|i| (i.sort_index, i.created_at));
                let Some(it) = ready_first.into_iter().next() else {
                    return json_result(&serde_json::json!({
                        "ok": false,
                        "reason": "no ready non-epic item on thread",
                    }));
                };
                it
            }
        };

        let updated = self
            .services
            .tasks
            .update(
                target.id,
                oxplow_app::UpdateTaskChanges {
                    status: Some(oxplow_domain::TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| internal(e.to_string()))?;

        let prompt =
            compose_dispatch_brief(&updated, params.0.extra_context.as_deref().unwrap_or(""));
        self.emit_tasks_changed(updated.thread_id.clone());
        json_result(&serde_json::json!({
            "ok": true,
            "prompt": prompt,
            "itemId": updated.id,
        }))
    }

    #[tool(
        description = "Branch a new thread off an existing one (shared stream, fresh thread row)."
    )]
    async fn fork_thread(
        &self,
        params: Parameters<ForkThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        expect_id_kind(
            "fork_thread",
            "source_thread_id",
            &params.0.source_thread_id,
            ID_THREAD,
        )?;
        let source = ThreadId::from(params.0.source_thread_id);
        let parent = self
            .services
            .thread_store
            .get(&source)
            .await
            .map_err(internal)?
            .ok_or_else(|| McpError::invalid_params("source thread not found", None))?;
        let child = self
            .services
            .threads
            .create(&parent.stream_id, params.0.title, parent.pane_target)
            .await
            .map_err(|e| internal(e.to_string()))?;
        json_result(&child)
    }

    #[tool(
        description = "Unified backlinks: every page (wiki, task, commit, finding, \
                       …) that points AT the given target page. The target is identified \
                       by `kind` (e.g. \"file\", \"wiki\", \"task\", \"git-commit\", \
                       \"finding\", \"directory\") and `id` (path / slug / wi-… / sha / id). \
                       Returns one row per inbound edge, including ref_type so the caller \
                       can distinguish e.g. a commit's touched_file edge from a wiki body \
                       mention."
    )]
    async fn list_backlinks(
        &self,
        params: Parameters<PageRefParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .services
            .page_ref_store
            .list_backlinks(&p.kind, &p.id, Some(p.limit as i64))
            .await
            .map_err(internal)?;
        json_result(&edges)
    }

    #[tool(
        description = "Unified outbound: every page the given source page points AT. \
                       Inverse of `list_backlinks` — ask \"what does THIS page reference?\". \
                       Same `kind`/`id` shape as list_backlinks."
    )]
    async fn list_outbound(
        &self,
        params: Parameters<PageRefParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let edges = self
            .services
            .page_ref_store
            .list_outbound(&p.kind, &p.id, Some(p.limit as i64))
            .await
            .map_err(internal)?;
        json_result(&edges)
    }

    #[tool(
        description = "Wiki pages that reference the given note slug in their related_notes \
                       (from [[other-note-slug]] wikilinks). Use for note-to-note backlinks."
    )]
    async fn find_wiki_pages_for_wiki_page(
        &self,
        params: Parameters<FindNotesForNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut hits = oxplow_app::wiki_pages::backlinks_for_note(
            &self.services.wiki_page_store,
            &params.0.slug,
        )
        .await
        .map_err(internal)?;
        if (params.0.limit as usize) > 0 && hits.len() > params.0.limit as usize {
            hits.truncate(params.0.limit as usize);
        }
        json_result(&hits)
    }

    // ---------- LSP ----------

    #[tool(description = "LSP textDocument/definition for a position in a file.")]
    async fn lsp_definition(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/definition",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/hover for a position in a file.")]
    async fn lsp_hover(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/references for a position in a file.")]
    async fn lsp_references(
        &self,
        params: Parameters<LspPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                    "position": { "line": p.line, "character": p.character },
                    "context": { "includeDeclaration": true },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "LSP textDocument/diagnostic — pulls the latest diagnostics for a file.")]
    async fn lsp_diagnostics(
        &self,
        params: Parameters<LspDiagnosticsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let proxy: std::sync::Arc<oxplow_app::LspProxy> =
            resolve_lsp_proxy(&self.services, &p.stream_id, &p.language).await?;
        let resp = proxy
            .request(
                "textDocument/diagnostic",
                serde_json::json!({
                    "textDocument": { "uri": p.uri },
                }),
            )
            .await
            .map_err(|e| internal(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            resp.to_string(),
        )]))
    }

    #[tool(description = "Re-read a wiki page's body file and refresh the FTS index.")]
    async fn resync_wiki_page(
        &self,
        params: Parameters<ResyncNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let slug = params.0.slug;
        let mut note = self
            .services
            .wiki_page_store
            .get(&slug)
            .await
            .map_err(internal)?
            .ok_or_else(|| McpError::invalid_params(format!("note not found: {slug}"), None))?;
        let body_path = self
            .services
            .layout
            .project_dir
            .join(".oxplow")
            .join("wiki")
            .join(format!("{slug}.md"));
        let body = std::fs::read_to_string(&body_path).unwrap_or_default();
        // Refresh excerpt + size; upsert re-syncs the FTS mirror.
        note.body_excerpt = body.chars().take(500).collect();
        note.body_size_bytes = body.len() as i64;
        note.updated_at = oxplow_domain::Timestamp::now();
        self.services
            .wiki_page_store
            .upsert(&note)
            .await
            .map_err(internal)?;
        json_result(&note)
    }
}

fn parse_status(s: &str) -> Result<TaskStatus, McpError> {
    Ok(match s {
        "ready" => TaskStatus::Ready,
        "in_progress" => TaskStatus::InProgress,
        "blocked" => TaskStatus::Blocked,
        "done" => TaskStatus::Done,
        "canceled" => TaskStatus::Canceled,
        "archived" => TaskStatus::Archived,
        other => {
            return Err(McpError::invalid_params(
                format!("unknown task status: {other}"),
                None,
            ))
        }
    })
}

fn parse_priority(s: &str) -> Result<oxplow_domain::TaskPriority, McpError> {
    use oxplow_domain::TaskPriority as P;
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

/// Resolve the per-(stream, language) LspProxy. Helper sitting
/// outside the `#[tool_router]` impl so the macro doesn't try to
/// route it as a tool.
async fn resolve_lsp_proxy(
    services: &Services,
    stream_id: &str,
    language: &str,
) -> Result<std::sync::Arc<oxplow_app::LspProxy>, McpError> {
    expect_id_kind("lsp", "stream_id", stream_id, ID_STREAM)?;
    let stream = services
        .streams
        .list_streams()
        .await
        .map_err(|e| internal(e.to_string()))?
        .into_iter()
        .find(|s| s.id.as_str() == stream_id)
        .ok_or_else(|| McpError::invalid_params(format!("stream not found: {stream_id}"), None))?;
    let cwd = std::path::PathBuf::from(&stream.worktree_path);
    services
        .lsp_sessions
        .ensure(stream_id, language, cwd)
        .await
        .map_err(|e| internal(e.to_string()))
}

fn parse_link_type(s: &str) -> Result<TaskLinkType, McpError> {
    Ok(match s {
        "blocks" => TaskLinkType::Blocks,
        "relates_to" => TaskLinkType::RelatesTo,
        "discovered_from" => TaskLinkType::DiscoveredFrom,
        "duplicates" => TaskLinkType::Duplicates,
        "supersedes" => TaskLinkType::Supersedes,
        "replies_to" => TaskLinkType::RepliesTo,
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
                "Oxplow MCP server. Exposes task, note, wiki, and stream surfaces. \
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

/// Validate that a caller-supplied id string carries the expected
/// `<prefix>-…` shape. When the prefix mismatches a known one, return
/// an `invalid_params` error that names the tool/parameter, the value
/// passed, the kind it was inferred to be, and the kind expected. This
/// converts opaque downstream FK-violation errors into actionable
/// guidance at the protocol boundary.
/// Parse a task id from its string form. Returns an error suitable for
/// returning straight from a tool handler when the input is not a
/// non-negative integer.
fn parse_task_id(tool: &str, param: &str, value: &str) -> Result<oxplow_domain::TaskId, McpError> {
    match oxplow_domain::TaskId::try_from_str(value) {
        Some(id) => Ok(id),
        None => Err(McpError::invalid_params(
            format!("{tool}: `{param}` expects a task id (integer), got `{value}`"),
            None,
        )),
    }
}

/// String-id prefix validator. Tasks now have integer ids and go
/// through [`parse_task_id`]; everything else still carries a
/// `<prefix>-<rest>` shape, and this helper confirms a caller-supplied
/// value matches the prefix the tool wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IdPrefix {
    pub prefix: &'static str,
    pub label: &'static str,
}

pub(crate) const ID_STREAM: IdPrefix = IdPrefix {
    prefix: "s-",
    label: "stream id (s-…)",
};
pub(crate) const ID_THREAD: IdPrefix = IdPrefix {
    prefix: "b-",
    label: "thread id (b-…)",
};
pub(crate) const ID_NOTE: IdPrefix = IdPrefix {
    prefix: "n-",
    label: "note id (n-…)",
};
pub(crate) const ID_FOLLOWUP: IdPrefix = IdPrefix {
    prefix: "fu-",
    label: "follow-up id (fu-…)",
};

fn expect_id_kind(
    tool: &str,
    param: &str,
    value: &str,
    expected: IdPrefix,
) -> Result<(), McpError> {
    if value.starts_with(expected.prefix) && value.len() > expected.prefix.len() {
        return Ok(());
    }
    // Tell the caller what the value *looks* like so they can correct
    // an "I passed a thread id where a stream id was expected" mix-up
    // without a second round-trip.
    let actual_label = match value.split_once('-') {
        Some(("s", _)) => "stream id (s-…)",
        Some(("b", _)) => "thread id (b-…)",
        Some(("n", _)) => "note id (n-…)",
        Some(("fu", _)) => "follow-up id (fu-…)",
        Some(("at", _)) => "agent-turn id (at-…)",
        Some(("he", _)) => "hook-event id (he-…)",
        Some(("ef", _)) => "effort id (ef-…)",
        Some(("pv", _)) => "page-visit id (pv-…)",
        Some(("ue", _)) => "usage-event id (ue-…)",
        Some(("bg", _)) => "background-task id (bg-…)",
        Some(_) => "id with an unrecognised prefix",
        None => "value with no `<prefix>-…` shape",
    };
    let msg = format!(
        "{tool}: `{param}` expects a {expected_label}, but got `{value}` which looks like a \
         {actual_label}",
        tool = tool,
        param = param,
        expected_label = expected.label,
        value = value,
    );
    Err(McpError::invalid_params(msg, None))
}

/// Compose the prompt the orchestrator passes to
/// `Agent(subagent_type='Explore', prompt=…)`. Pure so it's
/// testable without an MCP server. Mirrors `composeDelegateQueryPrompt`
/// from `src/mcp/mcp-tools.ts`.
fn compose_delegate_query_prompt(
    thread_id: &str,
    question: &str,
    focus: &str,
    note_id: &str,
) -> String {
    let mut parts: Vec<String> = vec![
        "You are an Explore subagent answering one focused exploration question for the orchestrator.".into(),
        String::new(),
        format!("threadId: {thread_id}"),
        format!("noteId: {note_id}"),
        String::new(),
        "## Question".into(),
        question.to_string(),
    ];
    if !focus.is_empty() {
        parts.push(String::new());
        parts.push("## Focus".into());
        parts.push(focus.to_string());
    }
    parts.push(String::new());
    parts.push("## How to report".into());
    parts.push(
        "When done, call `mcp__oxplow__record_query_finding({ noteId, body })` ONCE with your complete finding. \
         The body should be concise, structured prose — file paths, key function names, and the direct answer to the question. \
         Do not make code changes. Do not create tasks. Read/Grep/Glob only."
            .into(),
    );
    parts.join("\n")
}

/// Compose the brief the orchestrator passes to the general-purpose
/// Agent tool to dispatch a task to a subagent. Pure so it's
/// testable.
///
/// Sections: identity, description, AC, optional extra context, and
/// the closing reminder pointing at the subagent-protocol skill.
/// Per-item notes used to render here too but were retired —
/// task_effort.summary already records what shipped on prior
/// attempts; reviewers see it from the task activity timeline.
fn compose_dispatch_brief(item: &oxplow_domain::Task, extra_context: &str) -> String {
    let mut out: Vec<String> = vec![
        format!("Task: {}", item.title),
        format!("itemId: {}", item.id.value()),
        format!("priority: {:?}", item.priority),
        String::new(),
    ];
    if !item.description.is_empty() {
        out.push("## Description".into());
        out.push(item.description.clone());
        out.push(String::new());
    }
    if let Some(ac) = item.acceptance_criteria.as_deref() {
        if !ac.is_empty() {
            out.push("## Acceptance criteria".into());
            out.push(ac.to_string());
            out.push(String::new());
        }
    }
    if !extra_context.is_empty() {
        out.push("## Extra context".into());
        out.push(extra_context.to_string());
        out.push(String::new());
    }
    out.push("## Protocol".into());
    out.push(
        "Follow the `oxplow-subagent-work-protocol` skill: mark in_progress on entry; \
         done on exit. Return ONE line: `oxplow-result: {\"ok\":true,\"itemId\":\"<id>\",…}`. \
         Pass `touched_files` to `complete_task` so Local History attributes the writes."
            .into(),
    );
    out.join("\n")
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
    use oxplow_domain::stores::TaskStore;
    use oxplow_domain::task::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};
    use oxplow_domain::time::Timestamp;
    use rmcp::handler::server::wrapper::Parameters;

    fn boot() -> (tempfile::TempDir, Arc<Services>, OxplowMcp) {
        let project = tempfile::tempdir().unwrap();
        let services = Arc::new(Services::in_memory(project.path()).unwrap());
        let server = OxplowMcp::new(services.clone());
        (project, services, server)
    }

    /// Pull the first text block out of an MCP CallToolResult. Most
    /// of our handlers return a single JSON-encoded blob.
    fn text_payload(result: CallToolResult) -> String {
        for c in &result.content {
            if let Some(text) = c.as_text() {
                return text.text.clone();
            }
        }
        panic!("CallToolResult had no text content");
    }

    fn make_task(thread_id: Option<ThreadId>, title: &str) -> Task {
        let now = Timestamp::now();
        Task {
            id: TaskId::placeholder(),
            thread_id,
            parent_id: None,
            title: title.into(),
            description: String::new(),
            acceptance_criteria: None,
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
            category: None,
            tags: None,
        }
    }

    #[tokio::test]
    async fn server_constructs() {
        let (_proj, _svc, _server) = boot();
    }

    #[tokio::test]
    async fn get_info_advertises_tool_capability() {
        let (_proj, _svc, server) = boot();
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }

    #[tokio::test]
    async fn ping_returns_pong() {
        let (_proj, _svc, server) = boot();
        let r = server.ping().await.unwrap();
        assert_eq!(text_payload(r), "pong");
    }

    #[tokio::test]
    async fn app_version_returns_cargo_version() {
        let (_proj, _svc, server) = boot();
        let r = server.app_version().await.unwrap();
        assert_eq!(text_payload(r), env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn list_streams_returns_empty_for_fresh_project() {
        // ensure_primary requires a real git repo; the in_memory
        // service uses a tempdir that isn't one, so we exercise the
        // empty path. Production startup wires through a real repo.
        let (_proj, _services, server) = boot();
        let r = server.list_streams().await.unwrap();
        let body = text_payload(r);
        assert_eq!(body.trim(), "[]");
    }

    #[tokio::test]
    async fn list_backlog_includes_unassigned_items() {
        let (_proj, services, server) = boot();
        let backlog_item = make_task(None, "do the thing");
        let id = services.task_store.insert(&backlog_item).await.unwrap();

        let r = server.list_backlog().await.unwrap();
        let body = text_payload(r);
        assert!(
            body.contains(&id.to_string()),
            "backlog item missing from result: {body}",
        );
        assert!(body.contains("do the thing"), "title missing: {body}");
    }

    #[tokio::test]
    async fn get_task_round_trips() {
        let (_proj, services, server) = boot();
        let item = make_task(None, "round trip");
        let id = services.task_store.insert(&item).await.unwrap();

        let r = server
            .get_task(Parameters(TaskIdParams { id: id.to_string() }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("round trip"), "unexpected body: {body}");
    }

    #[tokio::test]
    async fn delete_task_soft_deletes() {
        let (_proj, services, server) = boot();
        let item = make_task(None, "to delete");
        let id = services.task_store.insert(&item).await.unwrap();

        server
            .delete_task(Parameters(TaskIdParams { id: id.to_string() }))
            .await
            .unwrap();

        // Soft-deleted: list_backlog should no longer include it.
        let r = server.list_backlog().await.unwrap();
        let body = text_payload(r);
        assert!(
            !body.contains(&format!("\"id\":{}", id.value())),
            "soft-deleted item should not appear in backlog: {body}",
        );
    }

    #[tokio::test]
    async fn create_task_rejects_stream_id_passed_as_thread_id() {
        let (_proj, _svc, server) = boot();
        let err = server
            .create_task(Parameters(CreateTaskMcpParams {
                thread_id: Some("s-deadbeef".into()),
                backlog: false,
                title: "x".into(),
                description: None,
                acceptance_criteria: None,
                kind: None,
                priority: None,
                status: None,
                category: None,
                tags: None,
                parent_id: None,
                touched_files: None,
            }))
            .await
            .expect_err("should reject stream id passed as thread_id");
        let msg = err.message.to_string();
        assert!(msg.contains("create_task"), "tool name missing: {msg}");
        assert!(msg.contains("thread_id"), "param name missing: {msg}");
        assert!(msg.contains("s-deadbeef"), "value missing: {msg}");
        assert!(msg.contains("stream id"), "actual kind missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[tokio::test]
    async fn create_task_rejects_unrecognised_thread_id() {
        let (_proj, _svc, server) = boot();
        let err = server
            .create_task(Parameters(CreateTaskMcpParams {
                thread_id: Some("nonsense".into()),
                backlog: false,
                title: "x".into(),
                description: None,
                acceptance_criteria: None,
                kind: None,
                priority: None,
                status: None,
                category: None,
                tags: None,
                parent_id: None,
                touched_files: None,
            }))
            .await
            .expect_err("should reject unprefixed value");
        let msg = err.message.to_string();
        assert!(msg.contains("nonsense"), "value missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[tokio::test]
    async fn upsert_task_round_trips() {
        let (_proj, _services, server) = boot();
        let item = make_task(None, "via mcp");
        let json = serde_json::to_string(&item).unwrap();

        let r = server
            .upsert_task(Parameters(UpsertTaskParams { item_json: json }))
            .await
            .unwrap();
        let body = text_payload(r);
        assert!(body.contains("via mcp"), "upsert response: {body}");
        // Parse the response to learn the assigned id, then re-fetch.
        let stored: Task = serde_json::from_str(&body).expect("upsert returns task json");
        assert_ne!(stored.id.value(), 0, "insert must assign a non-zero id");

        let fetched = server
            .get_task(Parameters(TaskIdParams {
                id: stored.id.to_string(),
            }))
            .await
            .unwrap();
        let body = text_payload(fetched);
        assert!(body.contains("via mcp"), "fetched after upsert: {body}");
    }

    #[tokio::test]
    async fn list_wiki_pages_runs_against_empty_store() {
        let (_proj, _services, server) = boot();
        // No notes seeded — the tool should still respond with an
        // empty-list payload rather than erroring.
        let r = server.list_wiki_pages().await.unwrap();
        let body = text_payload(r);
        assert_eq!(body.trim(), "[]");
    }

    // ---- Pure helpers: parse_status / parse_priority / parse_link_type ----

    #[test]
    fn parse_status_accepts_every_status() {
        assert!(matches!(parse_status("ready"), Ok(TaskStatus::Ready)));
        assert!(matches!(
            parse_status("in_progress"),
            Ok(TaskStatus::InProgress)
        ));
        assert!(matches!(parse_status("blocked"), Ok(TaskStatus::Blocked)));
        assert!(matches!(parse_status("done"), Ok(TaskStatus::Done)));
        assert!(matches!(parse_status("canceled"), Ok(TaskStatus::Canceled)));
        assert!(matches!(parse_status("archived"), Ok(TaskStatus::Archived)));
    }

    #[test]
    fn parse_status_rejects_in_progress_with_dash() {
        // The contract says snake_case `in_progress`; clients writing
        // `in-progress` should get an actionable error rather than
        // being silently coerced.
        let err = parse_status("in-progress").unwrap_err();
        assert!(err.message.contains("in-progress"));
    }

    #[test]
    fn parse_priority_accepts_each_value() {
        use oxplow_domain::TaskPriority as P;
        assert!(matches!(parse_priority("low"), Ok(P::Low)));
        assert!(matches!(parse_priority("medium"), Ok(P::Medium)));
        assert!(matches!(parse_priority("high"), Ok(P::High)));
        assert!(matches!(parse_priority("urgent"), Ok(P::Urgent)));
    }

    #[test]
    fn parse_priority_unknown_errors() {
        let err = parse_priority("critical").unwrap_err();
        assert!(err.message.contains("critical"));
    }

    #[test]
    fn parse_link_type_accepts_every_relation() {
        use oxplow_domain::TaskLinkType as L;
        assert!(matches!(parse_link_type("blocks"), Ok(L::Blocks)));
        assert!(matches!(parse_link_type("relates_to"), Ok(L::RelatesTo)));
        assert!(matches!(
            parse_link_type("discovered_from"),
            Ok(L::DiscoveredFrom)
        ));
        assert!(matches!(parse_link_type("duplicates"), Ok(L::Duplicates)));
        assert!(matches!(parse_link_type("supersedes"), Ok(L::Supersedes)));
        assert!(matches!(parse_link_type("replies_to"), Ok(L::RepliesTo)));
    }

    #[test]
    fn parse_link_type_unknown_errors() {
        let err = parse_link_type("flubs").unwrap_err();
        assert!(err.message.contains("flubs"));
    }

    // ---- expect_id_kind ----

    #[test]
    fn expect_id_kind_accepts_matching_prefix() {
        assert!(expect_id_kind("tool", "thread_id", "b-abc123", ID_THREAD,).is_ok());
    }

    #[test]
    fn expect_id_kind_error_names_tool_param_value_and_kinds() {
        let err = expect_id_kind("create_task", "thread_id", "s-abc123", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("create_task"), "tool name missing: {msg}");
        assert!(msg.contains("thread_id"), "param name missing: {msg}");
        assert!(msg.contains("s-abc123"), "value missing: {msg}");
        assert!(msg.contains("stream id"), "actual label missing: {msg}");
        assert!(msg.contains("thread id"), "expected label missing: {msg}");
    }

    #[test]
    fn expect_id_kind_unrecognised_id_shape_errors() {
        // No `<prefix>-…` shape at all — should still be flagged.
        let err = expect_id_kind("tool", "id", "no-prefix-shape", ID_THREAD).unwrap_err();
        let msg = err.message.to_string();
        assert!(msg.contains("no-prefix-shape"), "value missing: {msg}");
    }

    // ---- compose_delegate_query_prompt ----

    #[test]
    fn delegate_query_prompt_contains_required_sections() {
        let s = compose_delegate_query_prompt("b-1", "Where is X?", "", "n-2");
        assert!(s.contains("threadId: b-1"));
        assert!(s.contains("noteId: n-2"));
        assert!(s.contains("## Question"));
        assert!(s.contains("Where is X?"));
        assert!(s.contains("record_query_finding"));
    }

    #[test]
    fn delegate_query_prompt_omits_focus_section_when_empty() {
        let s = compose_delegate_query_prompt("b-1", "Q", "", "n-1");
        assert!(!s.contains("## Focus"));
    }

    #[test]
    fn delegate_query_prompt_includes_focus_when_provided() {
        let s = compose_delegate_query_prompt("b-1", "Q", "look in src/foo.rs", "n-1");
        assert!(s.contains("## Focus"));
        assert!(s.contains("look in src/foo.rs"));
    }

    // ---- compose_dispatch_brief ----

    #[test]
    fn dispatch_brief_includes_identity_and_protocol() {
        let mut item = make_task(None, "ship the thing");
        item.description = String::new();
        item.acceptance_criteria = None;
        let s = compose_dispatch_brief(&item, "");
        assert!(s.contains("Task: ship the thing"));
        assert!(s.contains(&format!("itemId: {}", item.id.value())));
        assert!(s.contains("priority:"));
        assert!(s.contains("## Protocol"));
        assert!(!s.contains("## Description"));
        assert!(!s.contains("## Acceptance criteria"));
        assert!(!s.contains("## Extra context"));
    }

    #[test]
    fn dispatch_brief_includes_description_when_non_empty() {
        let mut item = make_task(None, "x");
        item.description = "do the thing carefully".into();
        let s = compose_dispatch_brief(&item, "");
        assert!(s.contains("## Description"));
        assert!(s.contains("do the thing carefully"));
    }

    #[test]
    fn dispatch_brief_includes_acceptance_criteria_when_non_empty() {
        let mut item = make_task(None, "x");
        item.acceptance_criteria = Some("- it works".into());
        let s = compose_dispatch_brief(&item, "");
        assert!(s.contains("## Acceptance criteria"));
        assert!(s.contains("it works"));
    }

    #[test]
    fn dispatch_brief_skips_empty_acceptance_criteria_string() {
        // Some callers pass Some(""), which should still be treated
        // as "no AC" rather than rendering an empty section header.
        let mut item = make_task(None, "x");
        item.acceptance_criteria = Some(String::new());
        let s = compose_dispatch_brief(&item, "");
        assert!(!s.contains("## Acceptance criteria"));
    }

    #[test]
    fn dispatch_brief_appends_extra_context_when_provided() {
        let item = make_task(None, "x");
        let s = compose_dispatch_brief(&item, "see also note n-7");
        assert!(s.contains("## Extra context"));
        assert!(s.contains("see also note n-7"));
    }

    // ---- default_limit ----

    #[test]
    fn default_limit_is_stable() {
        // The exact value is part of the MCP contract; a regression
        // here changes how much data clients receive by default.
        assert_eq!(default_limit(), 20);
    }
}
