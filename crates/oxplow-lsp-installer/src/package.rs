//! Parser for the subset of Mason `package.yaml` we consume.
//!
//! Mason packages have a rich schema (templating, version expressions,
//! per-target build steps). For the prototype we only extract:
//!   - `name`, `description`, `homepage`
//!   - `languages` (used for language-id matching)
//!   - `source.id` (the purl string — we only support `pkg:github/...`)
//!   - `source.asset` (per-target asset/bin pairs for github sources)
//!   - `bin` (the server-name → bin-template map)
//!
//! Anything else is dropped on the floor.

use serde::Deserialize;
use std::collections::HashMap;
use thiserror::Error;

use crate::target::Target;

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("yaml parse: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("package source `{0}` is not a supported scheme — only pkg:github/... is implemented")]
    UnsupportedScheme(String),
    #[error("package source id `{0}` is malformed")]
    MalformedSourceId(String),
    #[error("package has no asset entry for target `{0}`")]
    NoAssetForTarget(String),
}

/// Parsed Mason package definition (subset).
#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub languages: Vec<String>,
    pub source: RawSource,
    /// Server-name → bin-path-template map. Mason supports template
    /// substitution like `"{{source.asset.bin}}"`. We resolve those
    /// at install time once we know which asset matched.
    #[serde(default)]
    pub bin: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawSource {
    pub id: String,
    /// Per-target asset/bin entries. Only present on github-release
    /// sources; missing for npm/pypi/etc.
    #[serde(default)]
    pub asset: Vec<RawAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAsset {
    /// Either a single target string or a list (Mason allows both).
    /// We normalize at lookup time via [`asset_targets`].
    pub target: serde_yaml::Value,
    /// Asset filename pattern — may include `{{version}}` placeholder.
    pub file: serde_yaml::Value,
    /// Binary path inside the (possibly extracted) asset.
    #[serde(default)]
    pub bin: Option<String>,
}

/// Source-id breakdown. Only `Github { owner, repo, version }` is
/// honoured by the installer; other variants are exposed so callers can
/// surface a meaningful "not yet supported" message instead of a parse
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceId {
    Github {
        owner: String,
        repo: String,
        version: String,
    },
    Other {
        scheme: String,
        raw: String,
    },
}

impl Package {
    /// Parse a Mason package.yaml from a string.
    pub fn parse(yaml: &str) -> Result<Self, PackageError> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    /// Decode `source.id` into a structured form.
    pub fn source_id(&self) -> Result<SourceId, PackageError> {
        parse_purl(&self.source.id)
    }

    /// Find the asset entry whose `target` list contains the given
    /// host target. Returns the resolved file/bin pair with `{{version}}`
    /// already substituted.
    pub fn asset_for_target(&self, target: &Target) -> Result<ResolvedAsset, PackageError> {
        let SourceId::Github { version, .. } = self.source_id()? else {
            return Err(PackageError::UnsupportedScheme(self.source.id.clone()));
        };
        for asset in &self.source.asset {
            if asset_targets(&asset.target)
                .iter()
                .any(|t| t == target.as_str())
            {
                let file = scalar_string(&asset.file)
                    .ok_or_else(|| PackageError::MalformedSourceId(self.source.id.clone()))?;
                let resolved_file = file.replace("{{version}}", &version);
                return Ok(ResolvedAsset {
                    file: resolved_file,
                    bin: asset.bin.clone(),
                });
            }
        }
        Err(PackageError::NoAssetForTarget(target.as_str().to_string()))
    }
}

/// One asset row resolved for the current host. `file` has had
/// `{{version}}` already substituted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub file: String,
    pub bin: Option<String>,
}

fn asset_targets(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

fn scalar_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            seq.iter().find_map(|x| x.as_str().map(|s| s.to_string()))
        }
        _ => None,
    }
}

fn parse_purl(s: &str) -> Result<SourceId, PackageError> {
    // Format: pkg:<scheme>/<...>@<version>
    let rest = s
        .strip_prefix("pkg:")
        .ok_or_else(|| PackageError::MalformedSourceId(s.to_string()))?;
    let (scheme, body) = rest
        .split_once('/')
        .ok_or_else(|| PackageError::MalformedSourceId(s.to_string()))?;
    let (path, version) = body
        .rsplit_once('@')
        .ok_or_else(|| PackageError::MalformedSourceId(s.to_string()))?;
    if scheme == "github" {
        let (owner, repo) = path
            .split_once('/')
            .ok_or_else(|| PackageError::MalformedSourceId(s.to_string()))?;
        return Ok(SourceId::Github {
            owner: owner.to_string(),
            repo: repo.to_string(),
            version: version.to_string(),
        });
    }
    Ok(SourceId::Other {
        scheme: scheme.to_string(),
        raw: s.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_ANALYZER: &str = r#"
name: rust-analyzer
description: |
  rust-analyzer is an implementation of the Language Server Protocol for Rust.
homepage: https://github.com/rust-lang/rust-analyzer
languages: [Rust]
categories: [LSP]
source:
  id: pkg:github/rust-lang/rust-analyzer@2026-04-27
  asset:
    - target: linux_x64_gnu
      file: rust-analyzer-x86_64-unknown-linux-gnu.gz
      bin: rust-analyzer-x86_64-unknown-linux-gnu
    - target: darwin_arm64
      file: rust-analyzer-aarch64-apple-darwin.gz
      bin: rust-analyzer-aarch64-apple-darwin
    - target: win_x64
      file: rust-analyzer-x86_64-pc-windows-msvc.zip
      bin: rust-analyzer.exe
bin:
  rust-analyzer: "{{source.asset.bin}}"
"#;

    const GOPLS: &str = r#"
name: gopls
description: gopls
homepage: https://pkg.go.dev/golang.org/x/tools/gopls
languages: [Go]
categories: [LSP]
source:
  id: pkg:golang/golang.org/x/tools/gopls@v0.21.1
bin:
  gopls: golang:gopls
"#;

    #[test]
    fn parses_rust_analyzer() {
        let p = Package::parse(RUST_ANALYZER).expect("parse ok");
        assert_eq!(p.name, "rust-analyzer");
        assert_eq!(p.languages, vec!["Rust"]);
        assert_eq!(
            p.source_id().unwrap(),
            SourceId::Github {
                owner: "rust-lang".into(),
                repo: "rust-analyzer".into(),
                version: "2026-04-27".into(),
            }
        );
        let asset = p
            .asset_for_target(&Target("darwin_arm64".into()))
            .expect("asset for darwin_arm64");
        assert_eq!(asset.file, "rust-analyzer-aarch64-apple-darwin.gz");
        assert_eq!(
            asset.bin.as_deref(),
            Some("rust-analyzer-aarch64-apple-darwin")
        );
    }

    #[test]
    fn parses_gopls_as_other_scheme() {
        let p = Package::parse(GOPLS).expect("parse ok");
        let SourceId::Other { scheme, .. } = p.source_id().unwrap() else {
            panic!("expected Other");
        };
        assert_eq!(scheme, "golang");
    }

    #[test]
    fn missing_target_errors() {
        let p = Package::parse(RUST_ANALYZER).unwrap();
        let err = p
            .asset_for_target(&Target("freebsd_x64".into()))
            .unwrap_err();
        assert!(matches!(err, PackageError::NoAssetForTarget(_)));
    }
}
