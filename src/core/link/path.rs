//! Pure path and classification helpers for agent linking.
//!
//! `classify` decides which entries of an agent skills dir are skills vs. other
//! files (the rules link, migrate and `agent --status` share); the rest are
//! symlink / path comparison utilities. No backup-side effects live here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{Agent, Env};

pub(crate) fn entry_name(entry: &fs::DirEntry) -> String {
    entry.file_name().to_string_lossy().into_owned()
}

/// Whether the agent's root dir exists in this scope (project: first component of
/// `skills_dir`; global: parent of the agent's skills dir).
pub(crate) fn agent_root_exists(agent: &Agent, global: bool, env: &Env, agent_dir: &Path) -> bool {
    if global {
        agent_dir.parent().map(|p| p.exists()).unwrap_or(false)
    } else {
        let root = agent.skills_dir.split('/').next().unwrap_or("");
        env.cwd.join(root).exists()
    }
}

/// Whether `link` (a symlink) resolves to `target`.
pub(crate) fn points_to(link: &Path, target: &Path) -> bool {
    match fs::read_link(link) {
        Ok(raw) => {
            let resolved = if raw.is_absolute() {
                raw
            } else {
                link.parent().unwrap_or(Path::new(".")).join(raw)
            };
            same_path(&resolved, target)
        }
        Err(_) => false,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => normalize_lexical(a) == normalize_lexical(b),
    }
}

/// Lexically resolve `.`/`..` components (no filesystem access).
fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether a dir entry is an old-model per-skill symlink into the canonical dir.
fn is_legacy_link(entry: &fs::DirEntry, canonical: &Path) -> bool {
    let path = entry.path();
    let is_symlink = fs::symlink_metadata(&path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !is_symlink {
        return false;
    }
    match fs::read_link(&path) {
        Ok(raw) => {
            let resolved = if raw.is_absolute() {
                raw
            } else {
                path.parent().unwrap_or(Path::new(".")).join(raw)
            };
            let canon = canonical
                .canonicalize()
                .unwrap_or_else(|_| normalize_lexical(canonical));
            let res = resolved
                .canonicalize()
                .unwrap_or_else(|_| normalize_lexical(&resolved));
            res == canon || res.starts_with(&canon)
        }
        Err(_) => false,
    }
}

/// Split dir entries into `(skills, others)`: skills are real subdirs and
/// symlinks whose target is a directory (e.g. links into a skills hub); others
/// are files and symlinks to non-directories. Old-model per-skill links into
/// the canonical dir are excluded from both (their content already lives there).
pub(crate) fn classify(entries: &[fs::DirEntry], canonical: &Path) -> (Vec<String>, Vec<String>) {
    let mut skills = Vec::new();
    let mut others = Vec::new();
    for entry in entries {
        if is_legacy_link(entry, canonical) {
            continue;
        }
        let name = entry_name(entry);
        if fs::metadata(entry.path())
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            skills.push(name);
        } else {
            others.push(name);
        }
    }
    (skills, others)
}
