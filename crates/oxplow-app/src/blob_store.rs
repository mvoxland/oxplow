//! Content-addressed blob store for snapshot bytes.
//!
//! Keyed by lowercase-hex SHA-256. Files land under
//! `<root>/<aa>/<full-hash>` (two-character shard prefix to keep
//! directory entry counts manageable on filesystems that don't
//! love huge flat dirs). Writes are idempotent — if the blob
//! already exists the bytes aren't rewritten.

use std::path::PathBuf;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlobStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob not found: {0}")]
    NotFound(String),
}

/// Cheap to clone — every method takes `&self` and reads the root
/// path on demand.
#[derive(Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Hex SHA-256 of `bytes`. Public so callers can compute a
    /// blob id without going through `write`.
    pub fn hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn path_for(&self, hash: &str) -> PathBuf {
        let shard = if hash.len() >= 2 { &hash[0..2] } else { "00" };
        self.root.join(shard).join(hash)
    }

    /// Persist `bytes` under their content hash and return the hash.
    /// Idempotent — re-writing the same bytes is a no-op (the file's
    /// mtime is bumped but contents are unchanged).
    pub fn write(&self, bytes: &[u8]) -> Result<String, BlobStoreError> {
        let hash = Self::hash(bytes);
        let path = self.path_for(&hash);
        if path.exists() {
            return Ok(hash);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write to a sibling temp then rename so partial-failure
        // never leaves a half-written blob in place.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    /// Read the bytes for `hash`. Errors with `NotFound` if the
    /// blob isn't on disk (pruned, never captured, etc).
    pub fn read(&self, hash: &str) -> Result<Vec<u8>, BlobStoreError> {
        let path = self.path_for(hash);
        if !path.exists() {
            return Err(BlobStoreError::NotFound(hash.to_string()));
        }
        Ok(std::fs::read(&path)?)
    }

    /// True iff a blob with `hash` exists on disk.
    pub fn has(&self, hash: &str) -> bool {
        self.path_for(hash).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let hash = store.write(b"hello").unwrap();
        let got = store.read(&hash).unwrap();
        assert_eq!(got, b"hello");
    }

    #[test]
    fn idempotent_write_for_same_bytes() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let h1 = store.write(b"x").unwrap();
        let h2 = store.write(b"x").unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn read_unknown_hash_errors() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let err = store.read("deadbeef").unwrap_err();
        assert!(matches!(err, BlobStoreError::NotFound(_)));
    }

    #[test]
    fn has_returns_true_after_write() {
        let dir = tempdir().unwrap();
        let store = BlobStore::new(dir.path().join("blobs"));
        let h = store.write(b"y").unwrap();
        assert!(store.has(&h));
        assert!(!store.has("deadbeef"));
    }
}
