//! Mason target-string mapping for the current host.
//!
//! Mason package.yaml asset entries key on strings like
//! `linux_x64_gnu`, `darwin_arm64`, `win_x64`. We only need to map the
//! current host to the right one — full cross-platform planning is out
//! of scope for the installer.

use std::fmt;

/// A Mason `target` string the asset table keys on. Stored as the raw
/// Mason string so it round-trips through serde_yaml without a custom
/// deserializer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target(pub String);

impl Target {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Best-effort host → Mason target. We pick the GNU variant on Linux —
/// musl support is rare in Mason packages and the installer can fall
/// back if the GNU asset is missing.
pub fn current_target() -> Target {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let s = match (os, arch) {
        ("linux", "x86_64") => "linux_x64_gnu",
        ("linux", "aarch64") => "linux_arm64_gnu",
        ("macos", "x86_64") => "darwin_x64",
        ("macos", "aarch64") => "darwin_arm64",
        ("windows", "x86_64") => "win_x64",
        ("windows", "aarch64") => "win_arm64",
        _ => "unknown",
    };
    Target(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_target_is_known_for_test_host() {
        let t = current_target();
        assert_ne!(
            t.as_str(),
            "unknown",
            "test host {} {} unmapped",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }

    #[test]
    fn target_round_trips_through_as_str() {
        let t = Target("linux_x64_gnu".into());
        assert_eq!(t.as_str(), "linux_x64_gnu");
        assert_eq!(t, Target("linux_x64_gnu".into()));
        assert_ne!(t, Target("darwin_arm64".into()));
    }

    #[test]
    fn target_display_writes_raw_string() {
        let t = Target("win_x64".into());
        assert_eq!(format!("{t}"), "win_x64");
    }
}
