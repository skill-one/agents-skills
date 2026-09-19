//! Directory-level agent links: connect each agent's skills dir to the canonical dir.
//!
//! The canonical dir holds the only real copies of installed skills; agents that do
//! not natively read it are integrated with a directory-level symlink
//! ([`link_agent`]): each agent's own skills dir becomes a relative link pointing
//! at the canonical dir, so every install/remove is instantly visible to all
//! linked agents.
//!
//! Linking adopts whatever the agent dir already holds: skill dirs are moved into
//! the canonical dir, non-skill entries are quarantined under
//! `.misc/<agent>/` inside it, and name clashes are dropped in favour of the
//! existing canonical (or disabled) copy. Adopted content is *not* restored by
//! unlink — it is managed by `remove`/`disable` from then on. The public result
//! enum lives in `outcome`, and path/classification helpers in `path`.

pub use crate::core::link::outcome::LinkOutcome;

mod outcome;
mod path;
#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{
    Agent, Env, agent_skills_dir, canonical_skills_dir, disabled_skills_dir, is_native,
};
use crate::core::install::sanitize_name;
use crate::core::link::path::{agent_root_exists, classify, entry_name, is_legacy_link, points_to};

/// Name of the quarantine dir for non-skill entries, inside the canonical dir.
///
/// The leading dot keeps it out of the skill namespace: install/discovery scans
/// skip dot-dirs, so quarantined files are never mistaken for installed skills.
pub(crate) const MISC_DIR_NAME: &str = ".misc";

/// Whether an agent's skills dir is linked to the canonical dir in the given
/// scope (a scope-native agent reads the canonical dir directly = always).
pub fn is_agent_linked(agent: &Agent, global: bool, env: &Env) -> bool {
    if is_native(agent, global, env) {
        return true;
    }
    match agent_skills_dir(agent, global, env) {
        Some(dir) => {
            fs::symlink_metadata(&dir)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
                && points_to(&dir, &canonical_skills_dir(global, env))
        }
        None => false,
    }
}

/// Link an agent's skills dir to the canonical dir (see [`LinkOutcome`] for cases).
///
/// Gating: non-native agents are skipped when their root dir does not exist in the
/// given scope (project: first path component of `skills_dir`, e.g. `.windsurf`;
/// global: the parent of the agent's skills dir, e.g. `~/.claude` or
/// `~/.gemini/config` for Antigravity) — this avoids fabricating agent presence.
/// `claude-code` is the historical exception: it is linked at project level even
/// when `.claude/` does not exist yet.
///
/// Content handling: an empty dir is replaced by the link directly; a non-empty
/// dir is adopted whole before linking — skill dirs move into the canonical dir,
/// non-skill entries into `.misc/<agent>/` inside it. Name clashes are dropped
/// (the canonical copy wins; a name disabled in `disabled-skills` stays disabled
/// and is not re-imported). Refusal is reserved for a foreign symlink.
pub fn link_agent(agent: &Agent, global: bool, env: &Env) -> LinkOutcome {
    // Scope-native agents use the canonical dir directly — nothing to link.
    // Note: an agent universal at project scope (e.g. Antigravity) may still
    // have a vendor-specific global dir that needs a real symlink.
    if is_native(agent, global, env) {
        return LinkOutcome::AlreadyLinked;
    }

    let Some(agent_dir) = agent_skills_dir(agent, global, env) else {
        // No resolvable dir in this scope/environment (e.g. an env-var-based
        // global dir whose variable is unset) — nothing to link here.
        return LinkOutcome::Skipped;
    };

    if !agent_root_exists(agent, global, env, &agent_dir) && agent.name != "claude-code" {
        return LinkOutcome::Skipped;
    }

    let canonical = canonical_skills_dir(global, env);

    match fs::symlink_metadata(&agent_dir) {
        // Missing: create the parent chain + a relative symlink.
        Err(_) => link_dir(&canonical, &agent_dir),

        Ok(meta) if meta.file_type().is_symlink() => {
            if points_to(&agent_dir, &canonical) {
                LinkOutcome::AlreadyLinked
            } else {
                LinkOutcome::Refused {
                    reason: format!(
                        "{} is a symlink pointing elsewhere; remove it first",
                        agent_dir.display()
                    ),
                }
            }
        }

        Ok(_) => {
            let entries = match fs::read_dir(&agent_dir) {
                Err(e) => {
                    return LinkOutcome::Failed {
                        error: e.to_string(),
                    };
                }
                Ok(rd) => rd.flatten().collect::<Vec<_>>(),
            };
            if entries.is_empty() {
                // Empty dir: safe to replace with the link.
                let _ = fs::remove_dir(&agent_dir);
                return link_dir(&canonical, &agent_dir);
            }

            let (adopted, quarantined, conflicts) = match adopt_all(
                agent,
                global,
                &canonical,
                &disabled_skills_dir(global, env),
                &entries,
            ) {
                Ok(triple) => triple,
                Err(error) => return LinkOutcome::Failed { error },
            };

            // What is left behind is legacy per-skill links only — links into the
            // canonical dir, whose content already lives there.
            if let Err(e) = fs::remove_dir_all(&agent_dir) {
                return LinkOutcome::Failed {
                    error: format!("remove {}: {e}", agent_dir.display()),
                };
            }
            match create_dir_symlink(&canonical, &agent_dir) {
                Ok(()) => LinkOutcome::Linked {
                    adopted,
                    quarantined,
                    conflicts,
                },
                Err(error) => LinkOutcome::Failed { error },
            }
        }
    }
}

/// Unlink an agent's skills dir from the canonical dir.
///
/// Returns a [`LinkOutcome`]: [`LinkOutcome::Unlinked`] on success,
/// [`LinkOutcome::NotLinked`] when there is nothing to do. Skills adopted on link
/// stay in the canonical dir; the agent dir is recreated empty. Foreign symlinks
/// and real dirs are left alone.
pub fn unlink_agent(agent: &Agent, global: bool, env: &Env) -> LinkOutcome {
    // Scope-native agents use the canonical dir directly — nothing to unlink.
    if is_native(agent, global, env) {
        return LinkOutcome::NotLinked;
    }

    let Some(agent_dir) = agent_skills_dir(agent, global, env) else {
        return LinkOutcome::NotLinked;
    };
    let canonical = canonical_skills_dir(global, env);

    match fs::symlink_metadata(&agent_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if !points_to(&agent_dir, &canonical) {
                // A foreign symlink: leave it alone.
                return LinkOutcome::NotLinked;
            }
            if let Err(e) = fs::remove_file(&agent_dir) {
                return LinkOutcome::Failed {
                    error: e.to_string(),
                };
            }
            // Recreate an empty dir so the agent does not see a missing skills dir.
            if let Err(e) = fs::create_dir_all(&agent_dir) {
                return LinkOutcome::Failed {
                    error: e.to_string(),
                };
            }
            LinkOutcome::Unlinked
        }
        _ => LinkOutcome::NotLinked,
    }
}

/// Classify the private content of an unlinked agent's skills dir:
/// `(skills, other entries)`, for `agent --status` reporting.
pub fn private_content(agent: &Agent, global: bool, env: &Env) -> (Vec<String>, Vec<String>) {
    let Some(dir) = agent_skills_dir(agent, global, env) else {
        return (Vec::new(), Vec::new());
    };
    // A symlinked skills dir is not private content (whatever it points at).
    if fs::symlink_metadata(&dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return (Vec::new(), Vec::new());
    }
    let Ok(rd) = fs::read_dir(&dir) else {
        return (Vec::new(), Vec::new());
    };
    let entries: Vec<_> = rd.flatten().collect();
    if entries.is_empty() {
        return (Vec::new(), Vec::new());
    }
    classify(&entries, &canonical_skills_dir(global, env))
}

/// `(adopted, quarantined, conflicts)` — the names moved into the canonical dir,
/// the non-skill names moved into `.misc/<agent>/`, and the names dropped.
type AdoptOutcome = (Vec<String>, Vec<String>, Vec<String>);

/// Move every entry of `agent_dir` into the canonical dir: skill dirs go to the
/// canonical root, non-skill entries into `.misc/<agent>/`. Name clashes are
/// dropped (an existing canonical or disabled copy wins), and legacy per-skill
/// links into the canonical dir are dropped outright.
///
/// Returns [`AdoptOutcome`].
fn adopt_all(
    agent: &Agent,
    global: bool,
    canonical: &Path,
    disabled: &Path,
    entries: &[fs::DirEntry],
) -> Result<AdoptOutcome, String> {
    let mut adopted = Vec::new();
    let mut quarantined = Vec::new();
    let mut conflicts = Vec::new();
    fs::create_dir_all(canonical).map_err(|e| format!("create {}: {e}", canonical.display()))?;

    // Created on first non-skill entry, so a clean dir never gains an empty `.misc`.
    let mut misc: Option<PathBuf> = None;

    for entry in entries {
        let name = entry_name(entry);
        let from = entry.path();

        // A legacy per-skill link already resolves into the canonical dir: its
        // content lives there, and moving the link in would make it self-referential.
        if is_legacy_link(entry, canonical) {
            conflicts.push(name);
            continue;
        }

        if fs::metadata(&from).map(|m| m.is_dir()).unwrap_or(false) {
            if canonical.join(&name).exists()
                || disabled.join(&name).exists()
                || disabled.join(sanitize_name(&name)).exists()
            {
                conflicts.push(name);
                continue;
            }
            let to = canonical.join(&name);
            fs::rename(&from, &to).map_err(|e| format!("move {}: {e}", from.display()))?;
            adopted.push(name);
        } else {
            let dir = match &misc {
                Some(dir) => dir.clone(),
                None => {
                    let root = canonical.join(MISC_DIR_NAME);
                    let dir = root.join(&agent.name);
                    fs::create_dir_all(&dir)
                        .map_err(|e| format!("create {}: {e}", dir.display()))?;
                    ensure_misc_gitignore(global, &root);
                    misc = Some(dir.clone());
                    dir
                }
            };
            let to = dir.join(&name);
            fs::rename(&from, &to).map_err(|e| format!("move {}: {e}", from.display()))?;
            quarantined.push(name);
        }
    }
    Ok((adopted, quarantined, conflicts))
}

/// Keep the quarantine dir out of version control in project scope: the canonical
/// dir itself is normally committed, but quarantined files are not skills.
fn ensure_misc_gitignore(global: bool, misc_root: &Path) {
    if global {
        return;
    }
    let gitignore = misc_root.join(".gitignore");
    if !gitignore.exists()
        && let Err(e) = fs::write(&gitignore, "*\n!.gitignore\n")
    {
        debug_assert!(false, "write {}: {e}", gitignore.display());
    }
}

/// Create the canonical symlink for an agent dir that is missing or empty.
fn link_dir(canonical: &Path, agent_dir: &Path) -> LinkOutcome {
    match create_dir_symlink(canonical, agent_dir) {
        Ok(()) => LinkOutcome::Linked {
            adopted: Vec::new(),
            quarantined: Vec::new(),
            conflicts: Vec::new(),
        },
        Err(error) => LinkOutcome::Failed { error },
    }
}

/// Create `link` as a symlink to `canonical`, using a relative target when possible.
fn create_dir_symlink(canonical: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return Err(format!("create {}: {e}", parent.display()));
    }
    let target = relative_target(canonical, link);
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(&target, link);
    result.map_err(|e| {
        format!(
            "symlink {} -> {}: {e} (on Windows, enable Developer Mode to allow symlinks)",
            link.display(),
            target.display()
        )
    })
}

/// Relative path from `link`'s parent to `canonical` (absolute fallback).
fn relative_target(canonical: &Path, link: &Path) -> PathBuf {
    let base = link.parent().unwrap_or(Path::new("."));
    pathdiff::diff_paths(canonical, base).unwrap_or_else(|| canonical.to_path_buf())
}
