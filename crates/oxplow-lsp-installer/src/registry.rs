//! Mason registry fetcher.
//!
//! Pulls `package.yaml` for individual packages from the
//! [`mason-org/mason-registry`](https://github.com/mason-org/mason-registry)
//! repository's `main` branch via raw.githubusercontent.com. Results
//! are cached on disk so the next call is offline.
//!
//! We intentionally do NOT pull the registry index (`registry.json.zip`)
//! up front — it's ~3 MB and 99% of the entries are unused. Per-package
//! lookup keeps the working set small.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use tracing::debug;

use crate::package::{Package, PackageError};

const RAW_BASE: &str = "https://raw.githubusercontent.com/mason-org/mason-registry/main/packages";

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("package: {0}")]
    Package(#[from] PackageError),
    #[error("package `{0}` not found in registry")]
    NotFound(String),
}

/// Fetcher with on-disk caching. `cache_dir` should be a long-lived
/// directory under the project (e.g. `.oxplow/lsp-cache/`).
pub struct Registry {
    cache_dir: PathBuf,
    http: reqwest::Client,
}

impl Registry {
    pub fn new(cache_dir: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("oxplow-lsp-installer/0.3")
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self { cache_dir, http }
    }

    /// Fetch and parse a package definition. Reads from disk cache when
    /// present; otherwise downloads and persists the YAML.
    pub async fn fetch_package(&self, name: &str) -> Result<Package, RegistryError> {
        let yaml = self.fetch_yaml(name).await?;
        Ok(Package::parse(&yaml)?)
    }

    async fn fetch_yaml(&self, name: &str) -> Result<String, RegistryError> {
        let cache_path = self.cache_path_for(name);
        if let Ok(s) = tokio::fs::read_to_string(&cache_path).await {
            debug!(package = name, "registry cache hit");
            return Ok(s);
        }
        let url = format!("{RAW_BASE}/{name}/package.yaml");
        debug!(package = name, %url, "registry fetch");
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(name.to_string()));
        }
        let resp = resp.error_for_status()?;
        let body = resp.text().await?;
        if let Some(parent) = cache_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&cache_path, &body).await?;
        Ok(body)
    }

    fn cache_path_for(&self, name: &str) -> PathBuf {
        // package names are lowercase ascii with hyphens; safe to use raw.
        self.cache_dir.join(name).join("package.yaml")
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn cache_hit_avoids_network() {
        let tmp = tempdir().unwrap();
        let reg = Registry::new(tmp.path().to_path_buf());
        // Seed the cache directly.
        let target_dir = tmp.path().join("rust-analyzer");
        tokio::fs::create_dir_all(&target_dir).await.unwrap();
        let yaml = "name: rust-analyzer\ndescription: x\nlanguages: [Rust]\nsource:\n  id: pkg:github/rust-lang/rust-analyzer@v1\n  asset: []\nbin: {}\n";
        tokio::fs::write(target_dir.join("package.yaml"), yaml).await.unwrap();
        let pkg = reg.fetch_package("rust-analyzer").await.unwrap();
        assert_eq!(pkg.name, "rust-analyzer");
    }
}
