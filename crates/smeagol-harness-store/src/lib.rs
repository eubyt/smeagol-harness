//! Markdown memory with workspace and project UUID namespaces.

use std::path::PathBuf;

mod storage;
pub use storage::{MemoryStore, ProjectNamespace};

/// Prefer an explicit data directory, then XDG or platform-specific user storage.
pub fn default_data_dir() -> PathBuf {
    if let Some(path) =
        std::env::var_os("SMEAGOL_HARNESS_HOME").or_else(|| std::env::var_os("ADRIAN_HARNESS_HOME"))
    {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(path).join("smeagol-harness");
    }
    #[cfg(windows)]
    {
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(path).join("SmeagolHarness");
        }
        if let Some(path) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(path).join("AppData/Local/SmeagolHarness");
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/smeagol-harness")
}
