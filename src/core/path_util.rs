//! Shared path comparison helpers.
//!
//! One home for the path utilities both the agent table (`agents`), the link
//! machinery (`link::path`) and the installer (`install`) need: lexical
//! normalization, lenient canonicalization for not-yet-created paths, and
//! containment/sameness checks. A fix lands in one place.

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

/// Canonicalize as much of `p` as exists: the deepest existing ancestor is
/// canonicalized and the not-yet-created tail appended. Unlike
/// `Path::canonicalize`, this also succeeds for paths that do not exist yet,
/// so an existing base and a to-be-created target resolve against the same
/// symlink-resolved root instead of comparing absolute vs raw paths.
pub(crate) fn canonicalize_lenient(p: &Path) -> PathBuf {
    let mut tail = PathBuf::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(resolved) = cur.canonicalize() {
            return resolved.join(&tail);
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail = PathBuf::from(name).join(&tail);
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Whether `target` is `base` or lives under it.
///
/// Paths are resolved leniently, so the check also works for targets that do
/// not exist yet.
pub(crate) fn path_contains(base: &Path, target: &Path) -> bool {
    let base_abs = canonicalize_lenient(base);
    let target_abs = canonicalize_lenient(target);
    target_abs == base_abs || target_abs.starts_with(&base_abs)
}

/// Whether `a` and `b` denote the same path: canonicalized when both resolve,
/// lexical fallback otherwise (for paths that do not exist yet).
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => normalize_lexical(a) == normalize_lexical(b),
    }
}
