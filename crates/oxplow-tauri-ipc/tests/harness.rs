//! Shared test harness for `#[tauri::command]` adapters.
//!
//! Builds a `tauri::App` with a mock runtime and a real
//! `oxplow_app::Services` over an in-memory DB, so each command's
//! body can be executed through the same `tauri::State<'_, AppState>`
//! plumbing the production shell uses. The aim is to cover the
//! argument-shape + error-mapping seam in
//! `crates/oxplow-tauri-ipc/src/commands/*` — store logic is
//! exercised in `oxplow-app` integration tests.

#![allow(dead_code)]

use std::sync::Arc;

use oxplow_app::Services;
use oxplow_tauri_ipc::AppState;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};
use tempfile::TempDir;

pub struct TestApp {
    pub _tmp: TempDir,
    pub state: AppState,
    pub app: App<MockRuntime>,
}

impl TestApp {
    pub fn build() -> Self {
        let tmp = TempDir::new().expect("tmp project dir");
        let services = Services::in_memory(tmp.path()).expect("services in_memory");
        let state: AppState = Arc::new(services);
        let app = mock_builder()
            .manage(state.clone())
            .build(mock_context(noop_assets()))
            .expect("mock app build");
        TestApp {
            _tmp: tmp,
            state,
            app,
        }
    }

    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}
