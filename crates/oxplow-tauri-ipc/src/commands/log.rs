use oxplow_git::{CommitDetail, GitLogCommit, GitLogOptions, GitLogResult};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_git_log(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    limit: Option<u32>,
    all: bool,
) -> Result<GitLogResult, IpcError> {
    let opts = GitLogOptions {
        limit: limit.map(|n| n as usize),
        all,
    };
    Ok(state.git.git_log(stream_id.as_deref(), opts).await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_commit_detail(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    sha: String,
) -> Result<Option<CommitDetail>, IpcError> {
    Ok(state.git.commit_detail(stream_id.as_deref(), sha).await)
}

#[tauri::command]
#[specta::specta]
pub async fn get_commits_ahead_of(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    base: String,
    head: String,
    limit: u32,
) -> Result<Vec<GitLogCommit>, IpcError> {
    Ok(state
        .git
        .commits_ahead_of(stream_id.as_deref(), base, head, limit as usize)
        .await)
}
