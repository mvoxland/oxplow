//! Github-release asset installer.
//!
//! Resolves a Mason package to a download URL, fetches it, and
//! extracts the binary into `<root>/<name>/`. Supports the asset
//! shapes that show up most often in Mason's github-release entries:
//! plain binary, gzip-of-binary, `.tar.gz`, and `.zip`. Anything else
//! returns [`InstallError::UnsupportedAsset`].

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::{debug, info};

use crate::package::{Package, PackageError, ResolvedAsset, SourceId};
use crate::target::Target;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("package error: {0}")]
    Package(#[from] PackageError),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("source scheme `{0}` not implemented (only pkg:github/... is supported)")]
    UnsupportedSource(String),
    #[error(
        "asset `{0}` has no extractor implemented (try .gz, .tar.gz, .zip, or a plain binary name)"
    )]
    UnsupportedAsset(String),
    #[error("package `{name}` declares no `bin` entry — nothing to install")]
    NoBin { name: String },
    #[error("after install, expected binary at {0:?} but it does not exist")]
    BinaryMissing(PathBuf),
}

/// Result of a successful install.
#[derive(Debug, Clone)]
pub struct Installed {
    /// Resolved absolute path to the installed binary (executable bit
    /// already set on Unix).
    pub binary: PathBuf,
    /// Mason package name.
    pub name: String,
    /// Languages the package serves (passed through from package.yaml).
    pub languages: Vec<String>,
    /// Version pulled from the source purl.
    pub version: String,
}

pub struct Installer {
    /// Root directory for installs. Each package gets its own subdir.
    /// Typical value: `<project>/.oxplow/lsp/`.
    root: PathBuf,
    http: reqwest::Client,
}

impl Installer {
    pub fn new(root: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("oxplow-lsp-installer/0.3")
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client");
        Self { root, http }
    }

    pub fn install_dir_for(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Download + extract `package` for the given host target.
    pub async fn install(
        &self,
        package: &Package,
        target: &Target,
    ) -> Result<Installed, InstallError> {
        let SourceId::Github {
            owner,
            repo,
            version,
        } = package.source_id()?
        else {
            let SourceId::Other { scheme, .. } = package.source_id()? else {
                unreachable!()
            };
            return Err(InstallError::UnsupportedSource(scheme));
        };
        let asset = package.asset_for_target(target)?;
        let url = format!(
            "https://github.com/{owner}/{repo}/releases/download/{version}/{file}",
            file = asset.file
        );
        info!(package = %package.name, %url, "downloading mason asset");
        let bytes = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let dest = self.install_dir_for(&package.name);
        if dest.exists() {
            tokio::fs::remove_dir_all(&dest).await?;
        }
        tokio::fs::create_dir_all(&dest).await?;
        let bytes_vec = bytes.to_vec();
        let asset_clone = asset.clone();
        let dest_clone = dest.clone();
        let pkg_name = package.name.clone();
        tokio::task::spawn_blocking(move || {
            extract_asset(&bytes_vec, &asset_clone, &dest_clone, &pkg_name)
        })
        .await
        .map_err(|e| InstallError::Io(std::io::Error::other(e.to_string())))??;

        // Resolve which file inside `dest` is the binary.
        let bin_path = resolve_bin_path(package, &asset, &dest)?;
        if !bin_path.exists() {
            return Err(InstallError::BinaryMissing(bin_path));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bin_path)?.permissions();
            perms.set_mode(perms.mode() | 0o111);
            std::fs::set_permissions(&bin_path, perms)?;
        }
        Ok(Installed {
            binary: bin_path,
            name: package.name.clone(),
            languages: package.languages.clone(),
            version,
        })
    }
}

fn extract_asset(
    bytes: &[u8],
    asset: &ResolvedAsset,
    dest: &Path,
    pkg_name: &str,
) -> Result<(), InstallError> {
    let file = &asset.file;
    if file.ends_with(".tar.gz") || file.ends_with(".tgz") {
        debug!(file, "extracting tar.gz");
        let gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest)?;
        return Ok(());
    }
    if file.ends_with(".zip") {
        debug!(file, "extracting zip");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        zip.extract(dest)?;
        return Ok(());
    }
    if file.ends_with(".gz") {
        // Plain gzip-of-binary. Mason's `bin` field tells us the output
        // name (without .gz). Decompress directly to that path.
        let bin_name = asset.bin.clone().unwrap_or_else(|| pkg_name.to_string());
        let out_path = dest.join(bin_name);
        let mut gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut buf = Vec::new();
        gz.read_to_end(&mut buf)?;
        let mut f = std::fs::File::create(&out_path)?;
        f.write_all(&buf)?;
        return Ok(());
    }
    // Plain binary with no recognized extension — treat as raw.
    if !file.contains('.') || file.ends_with(".exe") {
        let out_path = dest.join(file);
        let mut f = std::fs::File::create(&out_path)?;
        f.write_all(bytes)?;
        return Ok(());
    }
    Err(InstallError::UnsupportedAsset(file.clone()))
}

fn resolve_bin_path(
    package: &Package,
    asset: &ResolvedAsset,
    dest: &Path,
) -> Result<PathBuf, InstallError> {
    // Pick the first bin entry. Most LSP packages declare exactly one.
    let (_server_name, template) =
        package
            .bin
            .iter()
            .next()
            .ok_or_else(|| InstallError::NoBin {
                name: package.name.clone(),
            })?;
    // Resolve `{{source.asset.bin}}` to the asset's `bin` field.
    let resolved = if template.contains("{{source.asset.bin}}") {
        let bin = asset.bin.clone().unwrap_or_else(|| package.name.clone());
        template.replace("{{source.asset.bin}}", &bin)
    } else {
        template.clone()
    };
    Ok(dest.join(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Package;
    use std::io::Write;
    use tempfile::tempdir;

    fn make_package(yaml: &str) -> Package {
        Package::parse(yaml).expect("parse")
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn extract_gz_writes_decompressed_binary_to_dest() {
        let tmp = tempdir().unwrap();
        let asset = ResolvedAsset {
            file: "rust-analyzer-darwin.gz".into(),
            bin: Some("rust-analyzer-darwin".into()),
        };
        let bytes = gzip(b"fake-binary-contents");
        extract_asset(&bytes, &asset, tmp.path(), "rust-analyzer").unwrap();
        let out = tmp.path().join("rust-analyzer-darwin");
        assert!(out.exists());
        let read = std::fs::read(out).unwrap();
        assert_eq!(read, b"fake-binary-contents");
    }

    #[test]
    fn extract_zip_works() {
        use zip::write::SimpleFileOptions;
        let tmp = tempdir().unwrap();
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            zip.start_file("rust-analyzer.exe", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"win-binary").unwrap();
            zip.finish().unwrap();
        }
        let asset = ResolvedAsset {
            file: "rust-analyzer-win.zip".into(),
            bin: Some("rust-analyzer.exe".into()),
        };
        extract_asset(&buf, &asset, tmp.path(), "rust-analyzer").unwrap();
        assert!(tmp.path().join("rust-analyzer.exe").exists());
    }

    #[test]
    fn unsupported_source_surfaces_error() {
        let pkg = make_package(
            r#"
name: gopls
languages: [Go]
source:
  id: pkg:golang/golang.org/x/tools/gopls@v0.21.1
bin:
  gopls: golang:gopls
"#,
        );
        // Hit install via the source-id branch directly.
        let err = pkg.source_id().unwrap();
        match err {
            SourceId::Other { scheme, .. } => assert_eq!(scheme, "golang"),
            _ => panic!("expected Other"),
        }
    }

    #[test]
    fn resolve_bin_path_substitutes_template() {
        let pkg = make_package(
            r#"
name: rust-analyzer
languages: [Rust]
source:
  id: pkg:github/rust-lang/rust-analyzer@2026-04-27
  asset:
    - target: darwin_arm64
      file: rust-analyzer-aarch64-apple-darwin.gz
      bin: rust-analyzer-aarch64-apple-darwin
bin:
  rust-analyzer: "{{source.asset.bin}}"
"#,
        );
        let asset = pkg
            .asset_for_target(&Target("darwin_arm64".into()))
            .unwrap();
        let path = resolve_bin_path(&pkg, &asset, Path::new("/tmp/rust-analyzer")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/tmp/rust-analyzer/rust-analyzer-aarch64-apple-darwin")
        );
    }

    #[test]
    fn resolve_bin_path_passes_through_literal_template() {
        // Templates without `{{source.asset.bin}}` should be used
        // verbatim (joined onto dest). Useful for packages that ship
        // a fixed binary name independent of the per-target asset.
        let pkg = make_package(
            r#"
name: gopls
languages: [Go]
source:
  id: pkg:github/golang/tools@v0.21.1
  asset:
    - target: darwin_arm64
      file: gopls.tar.gz
bin:
  gopls: gopls
"#,
        );
        let asset = pkg
            .asset_for_target(&Target("darwin_arm64".into()))
            .unwrap();
        let path = resolve_bin_path(&pkg, &asset, Path::new("/tmp/gopls")).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/gopls/gopls"));
    }

    #[test]
    fn resolve_bin_path_no_bin_entry_errors() {
        let pkg = make_package(
            r#"
name: nobin
languages: [Foo]
source:
  id: pkg:github/example/nobin@v1.0
  asset:
    - target: darwin_arm64
      file: nobin.tar.gz
bin: {}
"#,
        );
        let asset = pkg
            .asset_for_target(&Target("darwin_arm64".into()))
            .unwrap();
        let err = resolve_bin_path(&pkg, &asset, Path::new("/tmp")).unwrap_err();
        match err {
            InstallError::NoBin { name } => assert_eq!(name, "nobin"),
            other => panic!("expected NoBin, got {other:?}"),
        }
    }

    #[test]
    fn extract_tar_gz_unpacks_archive_to_dest() {
        let tmp = tempdir().unwrap();
        // Build a tarball in-memory containing one file.
        let mut tar_buf: Vec<u8> = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            let payload = b"binary-bytes";
            header.set_path("gopls").unwrap();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, &payload[..]).unwrap();
            builder.into_inner().unwrap();
        }
        // Gzip the tarball.
        let mut gz_buf = Vec::new();
        {
            let mut enc =
                flate2::write::GzEncoder::new(&mut gz_buf, flate2::Compression::default());
            enc.write_all(&tar_buf).unwrap();
            enc.finish().unwrap();
        }
        let asset = ResolvedAsset {
            file: "gopls.tar.gz".into(),
            bin: None,
        };
        extract_asset(&gz_buf, &asset, tmp.path(), "gopls").unwrap();
        let extracted = tmp.path().join("gopls");
        assert!(extracted.exists());
        assert_eq!(std::fs::read(extracted).unwrap(), b"binary-bytes");
    }

    #[test]
    fn extract_plain_binary_no_extension_writes_file_named_for_asset() {
        // Some Mason entries point at a raw uncompressed binary with
        // no extension. The fallback branch should drop the bytes
        // straight into dest under the asset's `file` name.
        let tmp = tempdir().unwrap();
        let asset = ResolvedAsset {
            file: "lua-language-server".into(),
            bin: None,
        };
        extract_asset(b"raw-bytes", &asset, tmp.path(), "lua-language-server").unwrap();
        let out = tmp.path().join("lua-language-server");
        assert!(out.exists());
        assert_eq!(std::fs::read(out).unwrap(), b"raw-bytes");
    }

    #[test]
    fn extract_exe_writes_file_directly() {
        // Bare `.exe` (no zip wrapper) should land verbatim.
        let tmp = tempdir().unwrap();
        let asset = ResolvedAsset {
            file: "tool.exe".into(),
            bin: None,
        };
        extract_asset(b"win-bytes", &asset, tmp.path(), "tool").unwrap();
        let out = tmp.path().join("tool.exe");
        assert!(out.exists());
        assert_eq!(std::fs::read(out).unwrap(), b"win-bytes");
    }

    #[test]
    fn extract_unsupported_extension_errors() {
        let tmp = tempdir().unwrap();
        let asset = ResolvedAsset {
            file: "weird.xz".into(),
            bin: None,
        };
        let err = extract_asset(b"x", &asset, tmp.path(), "weird").unwrap_err();
        match err {
            InstallError::UnsupportedAsset(name) => assert_eq!(name, "weird.xz"),
            other => panic!("expected UnsupportedAsset, got {other:?}"),
        }
    }

    #[test]
    fn extract_gz_falls_back_to_pkg_name_when_bin_missing() {
        // When the asset has no `bin` field, the .gz branch uses the
        // package name as the output file name. Covers the
        // `unwrap_or_else(|| pkg_name.to_string())` branch.
        let tmp = tempdir().unwrap();
        let asset = ResolvedAsset {
            file: "rust-analyzer.gz".into(),
            bin: None,
        };
        extract_asset(&gzip(b"bytes"), &asset, tmp.path(), "rust-analyzer").unwrap();
        let out = tmp.path().join("rust-analyzer");
        assert!(out.exists());
    }
}
