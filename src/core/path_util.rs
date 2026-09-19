//! Shared lexical path helpers.
//!
//! These compare paths without touching the filesystem, so they work on
//! directories that do not exist yet. Both the agent table (`agents`) and the
//! link machinery (`link::path`) need the same normalization; keeping one copy
//! here means a fix lands in both.

use std::path::{Component, Path, PathBuf};

/// Lexically resolve `.` / `..` components for path comparison (no filesystem access).
pub fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
