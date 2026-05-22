//! Config mutation helpers.
//!
//! Reads/writes against `Services::config`. Each setter writes the
//! file back to disk before returning the new in-memory value.

use std::sync::{Arc, RwLock};

use oxplow_config::{write_project_config, ConfigError, OxplowConfig};

/// Returns a clone of the current in-memory config.
pub fn read_config(config: &Arc<RwLock<OxplowConfig>>) -> OxplowConfig {
    // Recover from poisoning rather than cascading the panic: a thread
    // that panicked while holding this lock leaves the config readable,
    // and the config is a plain data snapshot with no broken invariant.
    config.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Apply `mutate` to the config, persist to disk, and return the new
/// value. Holds the write lock for the duration of the mutation.
pub fn mutate_config(
    config: &Arc<RwLock<OxplowConfig>>,
    project_dir: &std::path::Path,
    mutate: impl FnOnce(&mut OxplowConfig),
) -> Result<OxplowConfig, ConfigError> {
    let mut guard = config.write().unwrap_or_else(|e| e.into_inner());
    mutate(&mut guard);
    write_project_config(project_dir, &guard)?;
    Ok(guard.clone())
}
