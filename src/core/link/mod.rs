//! Directory-level agent links: connect each agent's skills dir to the canonical dir.
//!
//! The canonical dir holds the only real copies of installed skills; agents that do
//! not natively read it are integrated with a directory-level symlink
//! ([`link_agent`]): each agent's own skills dir becomes a relative link pointing
//! at the canonical dir, so every install/remove is instantly visible to all
//! linked agents.
//!
//! Pre-existing content is never destroyed. A non-empty skills dir is parked
//! whole into the agent's backup slot (see `backup`); unlink restores it. The
//! public result enum lives in `outcome`, path/classification helpers in `path`,
//! and the unit tests in `tests`.

pub use crate::core::link::outcome::LinkOutcome;

mod backup;
mod outcome;
mod path;
#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{
    Agent, Env, agent_skills_dir, canonical_skills_dir, disabled_skills_dir, is_native,
};
use crate::core::link::backup::{
    PARKED_DIR_NAME, adopt_skills, backup_slot, cleanup_slot, migrate_from_backup, park_dir,
    parked_entries, restore_backup, rewrite_manifest,
};
use crate::core::link::path::{agent_root_exists, classify, entry_name, points_to};

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
/// Content handling: an empty dir is replaced by the link directly; any non-empty
/// dir is parked whole into the agent's backup slot (one atomic rename) before
/// linking. With `migrate`, skill dirs are then adopted into the canonical dir
/// (name clashes keep the canonical copy; names disabled in the
/// `disabled-skills` dir stay disabled). Refusal is reserved for a foreign
/// symlink or a previous backup that is still parked.
pub fn link_agent(agent: &Agent, global: bool, env: &Env, migrate: bool) -> LinkOutcome {
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
        Err(_) => map_link(
            create_dir_symlink(&canonical, &agent_dir),
            Vec::new(),
            Vec::new(),
            None,
        ),
        Ok(meta) if meta.file_type().is_symlink() => {
            if !points_to(&agent_dir, &canonical) {
                return LinkOutcome::Refused {
                    reason: format!(
                        "{} is a symlink pointing elsewhere; remove it first",
                        agent_dir.display()
                    ),
                };
            }
            // Already linked. With --migrate, pull parked skills out of the
            // backup slot into the canonical dir.
            if migrate {
                migrate_from_backup(agent, global, env, &canonical)
            } else {
                LinkOutcome::AlreadyLinked
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
                return map_link(
                    create_dir_symlink(&canonical, &agent_dir),
                    Vec::new(),
                    Vec::new(),
                    None,
                );
            }

            // Classification is for reporting and migrate decisions only — the
            // whole dir is parked either way.
            let (skills, others) = classify(&entries, &canonical);
            let names: Vec<String> = entries.iter().map(entry_name).collect();
            let slot = backup_slot(agent, global, env);
            let parked = slot.join(PARKED_DIR_NAME);
            if !parked_entries(&parked).is_empty() {
                return LinkOutcome::Refused {
                    reason: format!(
                        "a previous backup is still parked at {}; move it away or remove it before linking {}",
                        parked.display(),
                        agent_dir.display()
                    ),
                };
            }

            if let Some(failed) = park_dir(agent, global, env, &agent_dir, &names, &[]) {
                return failed;
            }
            let (moved, skipped) = if migrate {
                match adopt_skills(
                    &canonical,
                    &disabled_skills_dir(global, env),
                    &parked,
                    &skills,
                ) {
                    Ok(pair) => pair,
                    Err(error) => return LinkOutcome::Failed { error },
                }
            } else {
                (Vec::new(), Vec::new())
            };

            let res = create_dir_symlink(&canonical, &agent_dir);
            if migrate {
                let remaining: Vec<String> = names
                    .iter()
                    .filter(|n| !moved.contains(n))
                    .cloned()
                    .collect();
                let backup_dir = if remaining.is_empty() {
                    cleanup_slot(&slot);
                    None
                } else {
                    rewrite_manifest(&slot, agent, global, &moved, &remaining);
                    Some(parked)
                };
                match res {
                    Ok(()) => {
                        return LinkOutcome::Migrated {
                            moved,
                            skipped,
                            parked_others: others,
                            backup_dir,
                        };
                    }
                    Err(error) => return LinkOutcome::Failed { error },
                }
            }
            map_link(res, skills, others, Some(parked))
        }
    }
}

/// Unlink an agent's skills dir from the canonical dir, restoring the parked
/// dir (if any) with a single rename into its place.
///
/// Returns a [`LinkOutcome`]: [`LinkOutcome::Unlinked`] on success,
/// [`LinkOutcome::NotLinked`] when there is nothing to do. A real skills dir is
/// replaced only when a backup is pending and it is empty (or the restore fails
/// with a clear error); foreign symlinks are left alone.
pub fn unlink_agent(agent: &Agent, global: bool, env: &Env) -> LinkOutcome {
    // Scope-native agents use the canonical dir directly — nothing to unlink.
    if is_native(agent, global, env) {
        return LinkOutcome::NotLinked;
    }

    let Some(agent_dir) = agent_skills_dir(agent, global, env) else {
        return LinkOutcome::NotLinked;
    };
    let canonical = canonical_skills_dir(global, env);
    let slot = backup_slot(agent, global, env);
    let pending = !parked_entries(&slot.join(PARKED_DIR_NAME)).is_empty();

    match fs::symlink_metadata(&agent_dir) {
        // Dir gone: restore only when a backup is pending.
        Err(_) => {
            if pending {
                restore_backup(&slot, &agent_dir)
            } else {
                LinkOutcome::NotLinked
            }
        }
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
            restore_backup(&slot, &agent_dir)
        }
        Ok(_) => {
            if pending {
                restore_backup(&slot, &agent_dir)
            } else {
                LinkOutcome::NotLinked
            }
        }
    }
}

/// Classify the private content of an unlinked agent's skills dir:
/// `(skills, other entries)`, using the same rules as migrate.
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

/// The agent's parked dir with content, if any (for `--status`).
pub fn pending_backup(agent: &Agent, global: bool, env: &Env) -> Option<(PathBuf, Vec<String>)> {
    let parked = backup_slot(agent, global, env).join(PARKED_DIR_NAME);
    let entries = parked_entries(&parked);
    if entries.is_empty() {
        return None;
    }
    let items = entries.iter().map(entry_name).collect();
    Some((parked, items))
}

/// Map a symlink-creation result to a `Linked`/`Failed` outcome.
fn map_link(
    res: Result<(), String>,
    parked_skills: Vec<String>,
    parked_others: Vec<String>,
    backup_dir: Option<PathBuf>,
) -> LinkOutcome {
    match res {
        Ok(()) => LinkOutcome::Linked {
            parked_skills,
            parked_others,
            backup_dir,
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
