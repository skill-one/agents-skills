//! Private selection and lockfile helpers used by the `Manager` methods.
//!
//! Kept in their own module so the facade (`mod.rs`) stays focused on the methods
//! and their request/outcome structs (`types.rs`). Given `pub(crate)` so the
//! unit tests in `tests.rs` can exercise them directly.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::core::agents::{
    AGENTS, Agent, Env, detect_installed_agents, ensure_universal_agents, get_agent,
};
use crate::core::discover::Skill;
use crate::core::install::{move_skill, sanitize_name};
use crate::core::lock::{
    LockEntry, compute_folder_hash, global_lock_path, local_lock_path, lock_fields,
    read_local_lock, write_local_lock,
};
use crate::core::source::Source;
use crate::error::{Result, SkillsError};
use crate::manager::types::InstallSuccess;

/// Resolve agent selection: `"*"` → all; names → validated; empty → auto-detect + universal.
pub(crate) fn resolve_target_agents(names: &[String], env: &Env) -> Result<Vec<&'static Agent>> {
    if names.iter().any(|a| a == "*") {
        return Ok(AGENTS.iter().collect());
    }
    if !names.is_empty() {
        let mut agents = Vec::new();
        let mut invalid = Vec::new();
        for name in names {
            match get_agent(name) {
                Some(a) => agents.push(a),
                None => invalid.push(name.clone()),
            }
        }
        if !invalid.is_empty() {
            return Err(SkillsError::InvalidAgents(invalid.join(", ")));
        }
        return Ok(agents);
    }
    let installed = detect_installed_agents(env);
    Ok(ensure_universal_agents(installed))
}

/// Resolve requested names against available name sets, matching case-insensitively on
/// sanitized names. `sources` are consulted in order and the first available original
/// name wins — put higher-priority sources (e.g. lock keys) first.
fn resolve_names(requested: &[String], sources: &[&[String]]) -> Vec<String> {
    let mut identity: HashMap<String, String> = HashMap::new();
    for source in sources {
        for folder in *source {
            identity
                .entry(sanitize_name(folder))
                .or_insert_with(|| folder.clone());
        }
    }
    let mut matched = HashSet::new();
    for name in requested {
        if let Some(hit) = identity.get(&sanitize_name(name)) {
            matched.insert(hit.clone());
        }
    }
    let mut v: Vec<String> = matched.into_iter().collect();
    v.sort();
    v
}

/// Core enable/disable: resolve requested names, classify idempotent/missing, and move
/// dirs. Returns `(selected, already, missing)` where `selected` were actually moved.
///
/// `from_set` holds names in the current state (source of the move); `target_set` holds
/// names in the target state (to detect idempotent no-ops).
pub(crate) fn set_enabled_state(
    requested: &[String],
    from_set: &[String],
    target_set: &[String],
    global: bool,
    to_enabled: bool,
    env: &Env,
) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let selected = resolve_names(requested, &[from_set]);

    let mut already: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in requested {
        if selected
            .iter()
            .any(|s| sanitize_name(s) == sanitize_name(name))
        {
            continue;
        }
        if target_set
            .iter()
            .any(|d| sanitize_name(d) == sanitize_name(name))
        {
            already.push(name.clone());
        } else {
            missing.push(name.clone());
        }
    }
    already.sort();
    already.dedup();
    missing.sort();
    missing.dedup();

    for name in &selected {
        move_skill(name, global, to_enabled, env)?;
    }

    Ok((selected, already, missing))
}

/// Combine `--skill` args with the source's `@skill` filter into one selection list.
pub(crate) fn skill_filters(skills: &[String], skill_filter: Option<&str>) -> Vec<String> {
    let mut filters = skills.to_vec();
    if let Some(sf) = skill_filter {
        filters.push(sf.to_string());
    }
    filters
}

/// Resolve skill names to remove: match by sanitized name, lock keys take priority.
pub(crate) fn resolve_to_remove(
    requested: &[String],
    installed: &[String],
    disabled: &[String],
    lock_keys: &[String],
) -> Vec<String> {
    resolve_names(requested, &[lock_keys, installed, disabled])
}

pub(crate) fn lock_path(env: &Env, global: bool) -> PathBuf {
    if global {
        global_lock_path(&env.home)
    } else {
        local_lock_path(&env.cwd)
    }
}

pub(crate) fn write_lock(
    parsed: &Source,
    selected: &[Skill],
    successful: &[InstallSuccess],
    global: bool,
    env: &Env,
) -> Result<()> {
    let lock_path = lock_path(env, global);
    let mut lock = read_local_lock(&lock_path);
    lock.version = 1;

    let successful_names: HashSet<&str> = successful.iter().map(|s| s.name.as_str()).collect();
    for skill in selected {
        if !successful_names.contains(skill.name.as_str()) {
            continue;
        }
        let hash = compute_folder_hash(&skill.dir).unwrap_or_default();
        let (source, source_type, source_url, ref_, skill_path) = lock_fields(parsed);
        let mut entry = LockEntry::new(&source, &source_type, hash);
        entry.source_url = source_url;
        entry.r#ref = ref_;
        entry.skill_path = skill_path;
        lock.skills.insert(sanitize_name(&skill.name), entry);
    }
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_local_lock(&lock, &lock_path)
}
