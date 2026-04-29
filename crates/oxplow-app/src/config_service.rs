//! Config mutation helpers.
//!
//! Reads/writes against `Services::config`. Each setter writes the
//! file back to disk before returning the new in-memory value.

use std::sync::{Arc, RwLock};

use oxplow_config::{write_project_config, ConfigError, OxplowConfig};

/// Returns a clone of the current in-memory config.
pub fn read_config(config: &Arc<RwLock<OxplowConfig>>) -> OxplowConfig {
    config.read().expect("config rwlock").clone()
}

/// Apply `mutate` to the config, persist to disk, and return the new
/// value. Holds the write lock for the duration of the mutation.
pub fn mutate_config(
    config: &Arc<RwLock<OxplowConfig>>,
    project_dir: &std::path::Path,
    mutate: impl FnOnce(&mut OxplowConfig),
) -> Result<OxplowConfig, ConfigError> {
    let mut guard = config.write().expect("config rwlock");
    mutate(&mut guard);
    write_project_config(project_dir, &guard)?;
    Ok(guard.clone())
}
