//! Private selection helpers used by the `Manager` methods.
//!
//! Kept in their own module so the facade (`mod.rs`) stays focused on the methods
//! and their request/outcome structs (`types.rs`). Given `pub(crate)` so the
//! unit tests in `tests.rs` can exercise them directly.

use std::collections::{HashMap, HashSet};

use crate::core::agents::{AGENTS, Agent, Env, detect_installed_agents, get_agent};
use crate::core::install::{ScannedSkill, move_skill, remove_skill, sanitize_name};
use crate::error::{Result, SkillsError};

/// Resolve agent selection: `"*"` → all; names → validated; empty → auto-detect.
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

/// What [`apply_selection`] does with the skills its request matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionOp {
    /// Delete every on-disk copy (canonical + disabled) of each match.
    Remove,
    /// Move matches from the canonical dir into the disabled dir.
    Disable,
    /// Move matches from the disabled dir back into the canonical dir.
    Enable,
}

/// Whether `skill` and the requested `name` denote the same skill: matched
/// case-insensitively on the reported name (the SKILL.md frontmatter `name`),
/// folded with [`sanitize_name`] so an adopted skill's original spelling
/// (`PDF Master`) still matches its normalized identity (`pdf-master`). A
/// frontmatter name containing `/` is addressed by its folded form: slugify
/// drops `/` when building the slug, while the fold maps it to `-`.
fn same_skill(skill: &ScannedSkill, name: &str) -> bool {
    sanitize_name(&skill.name) == sanitize_name(name)
}

/// Resolve requested names against scanned skills; the first skill under a
/// folded key wins — `sources` are consulted in order, higher-priority first.
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

/// Core remove/disable/enable: resolve requested names against `sources`,
/// classify idempotent/missing against `target_set` (empty for remove, which
/// has no target state), and apply [`SelectionOp`].
///
/// Returns `(applied, already, missing)` where `applied` are the reported
/// names actually operated on; the moves/deletes themselves happen on the
/// real on-disk directories.
pub(crate) fn apply_selection(
    op: SelectionOp,
    requested: &[String],
    sources: &[&[ScannedSkill]],
    target_set: &[ScannedSkill],
    env: &Env,
) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let selected = resolve_names(requested, sources);

    let mut already: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in requested {
        if selected.iter().any(|s| same_skill(s, name)) {
            continue;
        }
        if target_set.iter().any(|d| same_skill(d, name)) {
            already.push(name.clone());
        } else {
            missing.push(name.clone());
        }
    }
    already.sort();
    already.dedup();
    missing.sort();
    missing.dedup();

    let mut applied = Vec::new();
    match op {
        SelectionOp::Remove => {
            for skill in &selected {
                if remove_skill(&skill.dir_name, env) {
                    applied.push(skill.name.clone());
                }
            }
        }
        SelectionOp::Disable | SelectionOp::Enable => {
            let to_enabled = op == SelectionOp::Enable;
            for skill in &selected {
                move_skill(&skill.dir_name, to_enabled, env)?;
                applied.push(skill.name.clone());
            }
        }
    }

    Ok((applied, already, missing))
}
