//! Wiki notes — file-backed knowledge base.

use oxplow_db::{WikiNote, WikiNoteSearchHit};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_wiki_notes(state: tauri::State<'_, AppState>) -> Result<Vec<WikiNote>, IpcError> {
    Ok(state.wiki_note_store.list().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_wiki_note(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<Option<WikiNote>, IpcError> {
    Ok(state.wiki_note_store.get(&slug).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn upsert_wiki_note(
    state: tauri::State<'_, AppState>,
    note: WikiNote,
) -> Result<(), IpcError> {
    Ok(state.wiki_note_store.upsert(&note).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_wiki_note(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<(), IpcError> {
    Ok(state.wiki_note_store.delete(&slug).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn search_wiki_titles(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<WikiNote>, IpcError> {
    Ok(state
        .wiki_note_store
        .search_titles(&query, limit as usize)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn search_wiki_bodies(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<WikiNoteSearchHit>, IpcError> {
    Ok(state
        .wiki_note_store
        .search_bodies(&query, limit as usize)
        .await?)
}
