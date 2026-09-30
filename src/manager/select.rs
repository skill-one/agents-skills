//! Private selection helpers used by the `Manager` methods.
//!
//! Kept in their own module so the facade (`mod.rs`) stays focused on the methods
//! and their request/outcome structs (`types.rs`). Given `pub(crate)` so the
//! unit tests in `tests.rs` can exercise them directly.

use std::collections::{HashMap, HashSet};

use crate::core::agents::{AGENTS, Agent, Env, detect_installed_agents, get_agent};
use crate::core::install::{ScannedSkill, move_skill, sanitize_name};
use crate::error::{Result, SkillsError};

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
    Ok(detect_installed_agents(env))
}

/// Resolve requested names against scanned skills, matching case-insensitively
/// on the reported name (the SKILL.md frontmatter `name`). `sources` are
/// consulted in order and the first skill under a folded key wins — put
/// higher-priority sources first.
fn resolve_names(requested: &[String], sources: &[&[ScannedSkill]]) -> Vec<ScannedSkill> {
    let mut identity: HashMap<String, ScannedSkill> = HashMap::new();
    for source in sources {
        for skill in *source {
            identity
                .entry(sanitize_name(&skill.name))
                .or_insert_with(|| skill.clone());
        }
    }
    let mut matched = HashSet::new();
    for name in requested {
        if let Some(hit) = identity.get(&sanitize_name(name)) {
            matched.insert(hit.clone());
        }
    }
    let mut v: Vec<ScannedSkill> = matched.into_iter().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Core enable/disable: resolve requested names, classify idempotent/missing, and move
/// dirs. Returns `(selected, already, missing)` where `selected` were actually moved.
///
/// `from_set` holds skills in the current state (source of the move); `target_set`
/// holds skills in the target state (to detect idempotent no-ops). Both selection
/// and the reported names use the skills' reported (frontmatter) names; the moves
/// themselves happen on the real on-disk directories.
pub(crate) fn set_enabled_state(
    requested: &[String],
    from_set: &[ScannedSkill],
    target_set: &[ScannedSkill],
    to_enabled: bool,
    env: &Env,
) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let selected = resolve_names(requested, &[from_set]);

    let mut already: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in requested {
        if selected
            .iter()
            .any(|s| sanitize_name(&s.name) == sanitize_name(name))
        {
            continue;
        }
        if target_set
            .iter()
            .any(|d| sanitize_name(&d.name) == sanitize_name(name))
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

    for skill in &selected {
        move_skill(&skill.dir_name, to_enabled, env)?;
    }

    let moved = selected.iter().map(|s| s.name.clone()).collect();
    Ok((moved, already, missing))
}

/// Resolve skill names to remove: match by reported (frontmatter) name.
pub(crate) fn resolve_to_remove(
    requested: &[String],
    installed: &[ScannedSkill],
    disabled: &[ScannedSkill],
) -> Vec<ScannedSkill> {
    resolve_names(requested, &[installed, disabled])
}
