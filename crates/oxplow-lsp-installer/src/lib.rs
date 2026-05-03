//! Mason-registry-backed LSP installer.
//!
//! Consumes package definitions from
//! [`mason-org/mason-registry`](https://github.com/mason-org/mason-registry)
//! to install language servers on demand. Mason packages each ship a
//! `package.yaml` describing where to fetch the binary; this crate parses
//! that subset, downloads the release asset for the current host, and
//! drops it under `.oxplow/lsp/<name>/` so callers (LspSessionManager)
//! can spawn it.
//!
//! Scope is intentionally narrow for the prototype:
//!   - `pkg:github/<owner>/<repo>@<version>` sources only.
//!   - Asset extraction handles plain binaries, gzip-of-binary, tar.gz,
//!     and zip. Other archive types return [`InstallError::UnsupportedAsset`].
//!   - No checksum verification beyond TLS — Mason's registry doesn't
//!     publish per-asset hashes.
//!   - No version pinning UI; whatever `source.id` says, that's what we
//!     fetch.
//!
//! Other source types (`pkg:npm/...`, `pkg:pypi/...`, `pkg:cargo/...`,
//! `pkg:golang/...`) return [`InstallError::UnsupportedSource`] for now.

mod installer;
mod package;
mod registry;
mod target;

pub use installer::{InstallError, Installer, Installed};
pub use package::{Package, PackageError, SourceId};
pub use registry::{Registry, RegistryError};
pub use target::{current_target, Target};
