//! Backup-slot machinery for directory-level agent links.
//!
//! When an agent's skills dir already has content, linking parks the whole dir
//! with a single atomic rename into the agent's backup slot
//! (`.agents/backup-skills/<agent>/skills`, next to a `manifest.json`), so unlinking
//! can restore it losslessly. `--migrate` adopts skill dirs out of the slot into
//! the canonical dir; skills disabled in the `disabled-skills` dir stay parked.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::core::agents::{Agent, Env, canonical_skills_dir, disabled_skills_dir};
use crate::core::install::sanitize_name;
use crate::core::link::outcome::LinkOutcome;
use crate::core::link::path::{classify, entry_name};

/// Name of the manifest file inside a backup slot.
const MANIFEST_NAME: &str = "manifest.json";

/// Name of the parked dir inside a backup slot (the agent's skills dir, renamed).
pub(crate) const PARKED_DIR_NAME: &str = "skills";

/// What a backup slot holds (written as `manifest.json` next to the parked dir).
#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    /// Agent whose skills dir was linked.
    agent: String,
    /// Scope of the link (`"project"` or `"global"`).
    scope: String,
    /// Unix timestamp (seconds) of the most recent park.
    created: u64,
    /// Top-level entries parked in the slot at park time.
    backed_up: Vec<String>,
    /// Skills since adopted into the canonical dir (no longer in the slot).
    migrated: Vec<String>,
}

/// Backup root for parked agent dirs: `(global ? home : cwd)/.agents/backup-skills`.
fn backup_root(global: bool, env: &Env) -> PathBuf {
    canonical_skills_dir(global, env)
        .parent()
        .unwrap_or(Path::new(".agents"))
        .join("backup-skills")
}

/// One agent's backup slot: `<backup root>/<agent name>`.
pub(crate) fn backup_slot(agent: &Agent, global: bool, env: &Env) -> PathBuf {
    backup_root(global, env).join(&agent.name)
}

/// Entries inside the parked dir (a missing dir is empty).
pub(crate) fn parked_entries(parked: &Path) -> Vec<fs::DirEntry> {
    fs::read_dir(parked)
        .map(|rd| rd.flatten().collect())
        .unwrap_or_default()
}

/// Rename the whole agent dir into the agent's backup slot and write the
/// manifest. `None` means success; `Some` is a [`LinkOutcome::Failed`].
pub(crate) fn park_dir(
    agent: &Agent,
    global: bool,
    env: &Env,
    agent_dir: &Path,
    names: &[String],
    migrated: &[String],
) -> Option<LinkOutcome> {
    let slot = backup_slot(agent, global, env);
    if let Err(e) = fs::create_dir_all(&slot) {
        return Some(LinkOutcome::Failed {
            error: format!("create {}: {e}", slot.display()),
        });
    }
    ensure_backup_gitignore(global, env);
    let parked = slot.join(PARKED_DIR_NAME);
    // A degenerate leftover (empty parked dir) does not block parking.
    let _ = fs::remove_dir(&parked);
    if let Err(e) = fs::rename(agent_dir, &parked) {
        return Some(LinkOutcome::Failed {
            error: format!("park {}: {e}", agent_dir.display()),
        });
    }
    write_manifest(&slot, agent, global, migrated, names);
    None
}

/// Move skill dirs out of the parked dir into the canonical dir. Name clashes keep
/// the canonical copy, and names parked in the disabled dir stay disabled (the
/// agent-side copy stays parked in both cases — a disabled skill must not be
/// re-imported). Returns `(moved, skipped)`.
pub(crate) fn adopt_skills(
    canonical: &Path,
    disabled: &Path,
    parked: &Path,
    skills: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut moved = Vec::new();
    let mut skipped = Vec::new();
    if skills.is_empty() {
        return Ok((moved, skipped));
    }
    fs::create_dir_all(canonical).map_err(|e| format!("create {}: {e}", canonical.display()))?;
    for name in skills {
        let from = parked.join(name);
        if canonical.join(name).exists()
            || disabled.join(name).exists()
            || disabled.join(sanitize_name(name)).exists()
        {
            skipped.push(name.clone());
        } else if let Err(e) = fs::rename(&from, canonical.join(name)) {
            return Err(format!("move {}: {e}", from.display()));
        } else {
            moved.push(name.clone());
        }
    }
    Ok((moved, skipped))
}

/// The agent dir is already linked to the canonical dir; with `--migrate`, pull
/// parked skill dirs out of the backup slot into the canonical dir. Name-clash
/// copies, skills disabled in the `disabled-skills` dir, non-skill entries and
/// old-model links stay parked.
pub(crate) fn migrate_from_backup(
    agent: &Agent,
    global: bool,
    env: &Env,
    canonical: &Path,
) -> LinkOutcome {
    let slot = backup_slot(agent, global, env);
    let parked = slot.join(PARKED_DIR_NAME);
    let entries = parked_entries(&parked);
    if entries.is_empty() {
        cleanup_slot(&slot);
        return LinkOutcome::AlreadyLinked;
    }
    let names: Vec<String> = entries.iter().map(entry_name).collect();
    let (skills, others) = classify(&entries, canonical);
    let (moved, skipped) = match adopt_skills(
        canonical,
        &disabled_skills_dir(global, env),
        &parked,
        &skills,
    ) {
        Ok(pair) => pair,
        Err(error) => return LinkOutcome::Failed { error },
    };
    let remaining: Vec<String> = names
        .iter()
        .filter(|n| !moved.contains(n))
        .cloned()
        .collect();
    if remaining.is_empty() {
        cleanup_slot(&slot);
        return LinkOutcome::Migrated {
            moved,
            skipped,
            parked_others: others,
            backup_dir: None,
        };
    }
    rewrite_manifest(&slot, agent, global, &moved, &remaining);
    LinkOutcome::Migrated {
        moved,
        skipped,
        parked_others: others,
        backup_dir: Some(parked),
    }
}

/// Restore the parked dir into `agent_dir` with a single atomic rename, then
/// drop the slot. Nothing parked → a fresh empty dir.
pub(crate) fn restore_backup(slot: &Path, agent_dir: &Path) -> LinkOutcome {
    let parked = slot.join(PARKED_DIR_NAME);
    let restored: Vec<String> = parked_entries(&parked).iter().map(entry_name).collect();
    if restored.is_empty() {
        cleanup_slot(slot);
        // Recreate an empty dir so the agent does not see a missing skills dir.
        if let Err(e) = fs::create_dir_all(agent_dir) {
            return LinkOutcome::Failed {
                error: e.to_string(),
            };
        }
        return LinkOutcome::Unlinked {
            restored: Vec::new(),
            restored_from: None,
        };
    }
    // The target must be gone (or an empty leftover dir).
    match fs::symlink_metadata(agent_dir) {
        Err(_) => {}
        Ok(m) if m.is_dir() => {
            let is_empty = fs::read_dir(agent_dir)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(false);
            if !is_empty {
                return LinkOutcome::Failed {
                    error: format!(
                        "restore blocked: {} exists and is not empty; move the backup at {} manually",
                        agent_dir.display(),
                        parked.display()
                    ),
                };
            }
            if let Err(e) = fs::remove_dir(agent_dir) {
                return LinkOutcome::Failed {
                    error: e.to_string(),
                };
            }
        }
        Ok(_) => {
            return LinkOutcome::Failed {
                error: format!("restore blocked: {} exists", agent_dir.display()),
            };
        }
    }
    if let Err(e) = fs::rename(&parked, agent_dir) {
        return LinkOutcome::Failed {
            error: format!("restore {}: {e}", parked.display()),
        };
    }
    cleanup_slot(slot);
    LinkOutcome::Unlinked {
        restored,
        restored_from: Some(parked),
    }
}

/// Remove the manifest, the parked dir, the slot itself, and the backup root
/// when this was the last slot (each step only succeeds when empty).
pub(crate) fn cleanup_slot(slot: &Path) {
    let _ = fs::remove_file(slot.join(MANIFEST_NAME));
    let _ = fs::remove_dir(slot.join(PARKED_DIR_NAME));
    let _ = fs::remove_dir(slot);
    if let Some(root) = slot.parent() {
        let _ = fs::remove_dir(root);
    }
}

/// Write the slot manifest recording what was parked vs. adopted.
fn write_manifest(
    slot: &Path,
    agent: &Agent,
    global: bool,
    migrated: &[String],
    backed_up: &[String],
) {
    let manifest = BackupManifest {
        agent: agent.name.to_string(),
        scope: if global { "global" } else { "project" }.to_string(),
        created: now_secs(),
        backed_up: backed_up.to_vec(),
        migrated: migrated.to_vec(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = fs::write(slot.join(MANIFEST_NAME), json);
    }
}

/// Rewrite the slot manifest after some parked skills were adopted out.
pub(crate) fn rewrite_manifest(
    slot: &Path,
    agent: &Agent,
    global: bool,
    moved_out: &[String],
    remaining: &[String],
) {
    let previous = fs::read_to_string(slot.join(MANIFEST_NAME))
        .ok()
        .and_then(|s| serde_json::from_str::<BackupManifest>(&s).ok());
    let mut migrated = previous.map(|m| m.migrated).unwrap_or_default();
    migrated.extend(moved_out.iter().cloned());
    write_manifest(slot, agent, global, &migrated, remaining);
}

/// Keep the project-scope backup root out of version control (the `.agents/`
/// dir itself is normally committed).
fn ensure_backup_gitignore(global: bool, env: &Env) {
    if global {
        return;
    }
    let root = backup_root(global, env);
    let gitignore = root.join(".gitignore");
    if !gitignore.exists()
        && let Err(e) =
            fs::create_dir_all(&root).and_then(|_| fs::write(&gitignore, "*\n!.gitignore\n"))
    {
        debug_assert!(false, "write backup .gitignore: {e}");
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
