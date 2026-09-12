//! Install skills into the canonical dir and list what's installed.
//!
//! The canonical dir (`(global ? home : cwd)/.agents/skills`) is the single source of
//! truth: [`install_skill`] writes real files there and nowhere else. Agent
//! integration is a separate concern handled by [`crate::core::link`]. Copies skip
//! metadata.json/.git/__pycache__/__pypackages__.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{
    AGENTS, Env, agent_skills_dir, canonical_skills_dir, disabled_skills_dir, is_native,
};
use crate::core::discover::{Skill, parse_skill_md};
use crate::error::Result;

/// Outcome of installing a single skill into the canonical dir.
#[derive(Debug)]
pub struct InstallResult {
    /// Whether the install succeeded.
    pub success: bool,
    /// Canonical directory of the skill.
    pub canonical_path: PathBuf,
    /// Whether the install was skipped (source already inside the canonical dir).
    pub skipped: bool,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Sanitize a directory name: lowercase → fold non-`[a-z0-9._]` to `-` → trim leading/trailing `.`/`-` → truncate to 255 → fallback.
pub fn sanitize_name(name: &str) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Collapse consecutive separators.
    let folded = {
        let mut out = String::new();
        let mut prev_dash = false;
        for c in s.chars() {
            if c == '-' {
                if !prev_dash {
                    out.push(c);
                }
                prev_dash = true;
            } else {
                out.push(c);
                prev_dash = false;
            }
        }
        out
    };
    s = folded;
    let trimmed = s.trim_matches(|c: char| c == '.' || c == '-').to_string();
    let mut result: String = trimmed.chars().take(255).collect();
    if result.is_empty() {
        result = "unnamed-skill".to_string();
    }
    result
}

/// Canonical path of a skill.
pub fn get_canonical_path(name: &str, global: bool, env: &Env) -> PathBuf {
    canonical_skills_dir(global, env).join(sanitize_name(name))
}

/// Canonicalize as much of `p` as exists: the deepest existing ancestor is
/// canonicalized and the not-yet-created tail appended. Unlike
/// `Path::canonicalize`, this also succeeds for paths that do not exist yet,
/// so an existing base and a to-be-created target resolve against the same
/// symlink-resolved root instead of comparing absolute vs raw paths.
fn canonicalize_lenient(p: &Path) -> PathBuf {
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

fn path_safe(base: &Path, target: &Path) -> bool {
    let base_abs = canonicalize_lenient(base);
    let target_abs = canonicalize_lenient(target);
    target_abs == base_abs || target_abs.starts_with(&base_abs)
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    path_safe(a, b) || path_safe(b, a)
}

/// Clean and recreate a directory.
fn clean_and_create(dir: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir)?;
    Ok(())
}

/// Recursively copy a directory, excluding metadata.json / .git / __pycache__ / __pypackages__,
/// dereferencing symlinks (copying target contents).
pub fn copy_directory(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let src_path = entry.path();
        let dest_path = dest.join(&name);

        let meta = entry.metadata()?;
        if meta.is_dir() {
            if name == ".git" || name == "__pycache__" || name == "__pypackages__" {
                continue;
            }
            copy_directory(&src_path, &dest_path)?;
        } else if meta.is_file() {
            if name == "metadata.json" {
                continue;
            }
            fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Install a single skill into the canonical dir (the only place real files live).
pub fn install_skill(skill: &Skill, global: bool, env: &Env) -> InstallResult {
    let skill_name = sanitize_name(&skill.name);
    let canonical_base = canonical_skills_dir(global, env);
    let canonical_dir = canonical_base.join(&skill_name);

    if !path_safe(&canonical_base, &canonical_dir) {
        return InstallResult {
            success: false,
            canonical_path: canonical_dir,
            skipped: false,
            error: Some("Invalid skill name: potential path traversal detected".to_string()),
        };
    }

    // Source already inside the canonical dir → skip (avoid deleting the source).
    if paths_overlap(&skill.dir, &canonical_dir) {
        return InstallResult {
            success: true,
            canonical_path: canonical_dir,
            skipped: true,
            error: None,
        };
    }

    if let Err(e) =
        clean_and_create(&canonical_dir).and_then(|_| copy_directory(&skill.dir, &canonical_dir))
    {
        return InstallResult {
            success: false,
            canonical_path: canonical_dir,
            skipped: false,
            error: Some(e.to_string()),
        };
    }

    InstallResult {
        success: true,
        canonical_path: canonical_dir,
        skipped: false,
        error: None,
    }
}

/// An installed skill (used by list).
#[derive(Debug)]
pub struct InstalledSkill {
    /// Skill name.
    pub name: String,
    /// Canonical directory path.
    pub canonical_path: PathBuf,
    /// `"project"` or `"global"`.
    pub scope: String,
    /// Agent names this skill is linked to.
    pub agents: Vec<String>,
}

/// Scan the canonical dir, listing installed skills (including agent visibility).
///
/// An agent "sees" a skill when its skills dir is linked to the canonical dir
/// (directory-level symlink) — which also covers legacy per-skill links, since a
/// dir-level link makes `base/<skill>` resolve inside the canonical dir.
pub fn list_installed_skills(
    env: &Env,
    global: bool,
    agent_filter: &[String],
) -> Vec<InstalledSkill> {
    let scope = if global { "global" } else { "project" };
    let canonical = canonical_skills_dir(global, env);
    let mut out: Vec<InstalledSkill> = Vec::new();

    let entries = match fs::read_dir(&canonical) {
        Ok(e) => e,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        let skill_dir = entry.path();
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Some(skill) = parse_skill_md(&skill_md) else {
            continue;
        };
        let mut agents: Vec<String> = Vec::new();
        for agent in AGENTS.iter() {
            if !agent_filter.is_empty() && !agent_filter.iter().any(|a| a == &agent.name) {
                continue;
            }
            // Scope-native agents read the canonical dir directly. Hidden
            // agents stay excluded from this group (they never did before).
            if is_native(agent, global, env) {
                if !agent.hidden {
                    agents.push(agent.name.clone());
                }
                continue;
            }
            let Some(base) = agent_skills_dir(agent, global, env) else {
                continue;
            };
            let candidate = base.join(sanitize_name(&skill.name));
            if candidate.exists() || base.join(entry.file_name()).exists() {
                agents.push(agent.name.to_string());
            }
        }
        out.push(InstalledSkill {
            name: skill.name,
            canonical_path: skill_dir,
            scope: scope.to_string(),
            agents,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Scan the canonical dir, collecting installed skill directory names.
pub fn scan_installed(env: &Env, global: bool) -> Vec<String> {
    let canonical = canonical_skills_dir(global, env);
    let mut v: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&canonical) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                v.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    v.sort();
    v
}

/// Scan the disabled dir, collecting disabled skill directory names.
pub fn scan_disabled(env: &Env, global: bool) -> Vec<String> {
    let disabled = disabled_skills_dir(global, env);
    let mut v: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&disabled) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                v.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    v.sort();
    v
}

/// List skills parked in the disabled dir. Agents list is empty — they're hidden.
pub fn list_disabled_skills(env: &Env, global: bool) -> Vec<InstalledSkill> {
    let scope = if global { "global" } else { "project" };
    let disabled = disabled_skills_dir(global, env);
    let mut out: Vec<InstalledSkill> = Vec::new();

    let entries = match fs::read_dir(&disabled) {
        Ok(e) => e,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        let skill_dir = entry.path();
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Some(skill) = parse_skill_md(&skill_md) else {
            continue;
        };
        out.push(InstalledSkill {
            name: skill.name,
            canonical_path: skill_dir,
            scope: scope.to_string(),
            agents: Vec::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Move a skill directory between the canonical dir and the disabled dir.
///
/// `to_enabled=true` moves `disabled-skills/<name>` → `skills/<name>` (enable);
/// `to_enabled=false` moves `skills/<name>` → `disabled-skills/<name>` (disable).
/// The target parent dir is created if needed.
pub fn move_skill(name: &str, global: bool, to_enabled: bool, env: &Env) -> Result<()> {
    let canonical = canonical_skills_dir(global, env).join(sanitize_name(name));
    let disabled = disabled_skills_dir(global, env).join(sanitize_name(name));
    let (from, to) = if to_enabled {
        (&disabled, &canonical)
    } else {
        (&canonical, &disabled)
    };
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(from, to)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::link::{LinkOutcome, link_agent};
    use crate::core::test_utils::{env_at, write_and_parse_skill};

    fn write_skill(dir: &Path, name: &str) -> Skill {
        write_and_parse_skill(dir, name)
    }

    #[test]
    fn sanitize_name_basic() {
        assert_eq!(sanitize_name("PDF Master"), "pdf-master");
        assert_eq!(
            sanitize_name("Git Review Before Commit"),
            "git-review-before-commit"
        );
        assert_eq!(sanitize_name("../evil"), "evil");
        assert_eq!(sanitize_name("  "), "unnamed-skill");
        assert_eq!(sanitize_name("A.B_c"), "a.b_c");
        assert_eq!(sanitize_name("-leading-trailing-"), "leading-trailing");
    }

    #[test]
    fn path_safe_resolves_not_yet_created_targets_consistently() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path().join("skills");
        std::fs::create_dir_all(&base).unwrap();

        // Existing target: both sides canonicalize directly.
        let existing = base.join("alpha");
        std::fs::create_dir_all(&existing).unwrap();
        assert!(path_safe(&base, &existing));

        // Not-yet-created target under an existing base: the base canonicalizes
        // to an absolute path while the target cannot — the naive fallback made
        // this comparison fail (absolute vs raw) and reject a valid name.
        assert!(path_safe(&base, &base.join("beta")));

        // Fully not-yet-created base and target still resolve to one root.
        assert!(path_safe(
            &tmp.path().join("a/b"),
            &tmp.path().join("a/b/c")
        ));

        // Traversal outside the base is still rejected.
        assert!(!path_safe(&base, &tmp.path().join("elsewhere")));
    }

    #[test]
    fn canonical_dir_project_and_global() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        assert_eq!(
            canonical_skills_dir(false, &env),
            tmp.path().join(".agents/skills")
        );
        assert_eq!(
            canonical_skills_dir(true, &env),
            tmp.path().join(".agents/skills")
        );
    }

    #[test]
    fn install_skill_writes_canonical_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");

        let r = install_skill(&skill, false, &env);
        assert!(r.success, "err={:?}", r.error);
        assert!(!r.skipped);
        assert!(tmp.path().join(".agents/skills/pdf/SKILL.md").exists());
        // No agent dirs are created by install.
        assert!(!tmp.path().join(".claude").exists());
        assert!(!tmp.path().join(".windsurf").exists());
    }

    #[test]
    fn install_skill_skips_when_source_overlaps_canonical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join(".agents/skills/pdf");
        let skill = write_skill(&src, "pdf");

        let r = install_skill(&skill, false, &env);
        assert!(r.success);
        assert!(r.skipped);
        assert!(src.join("SKILL.md").exists());
    }

    #[test]
    fn copy_directory_excludes_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::create_dir_all(src.join("__pycache__")).unwrap();
        fs::write(src.join("SKILL.md"), "x").unwrap();
        fs::write(src.join("metadata.json"), "x").unwrap();
        fs::write(src.join(".git").join("HEAD"), "x").unwrap();

        let dest = tmp.path().join("dest");
        copy_directory(&src, &dest).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(!dest.join("metadata.json").exists());
        assert!(!dest.join(".git").exists());
        assert!(!dest.join("__pycache__").exists());
    }

    #[test]
    fn list_installed_skills_finds_canonical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, false, &env);

        let installed = list_installed_skills(&env, false, &[]);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "pdf");
        assert_eq!(installed[0].scope, "project");
    }

    #[test]
    fn list_global_skills_distinguishes_native_and_linkable_agents() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut env = Env::new(
            tmp.path().join("home"),
            tmp.path().join("config"),
            tmp.path().join("project"),
        );
        env.set_probe_system_dirs(false);
        env.set_vars(std::collections::HashMap::new());

        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, true, &env);

        // Global scope: cline natively reads ~/.agents/skills; antigravity reads
        // ~/.gemini/config/skills and is not connected yet.
        let installed = list_installed_skills(&env, true, &[]);
        assert_eq!(installed.len(), 1);
        assert!(installed[0].agents.contains(&"cline".to_string()));
        assert!(!installed[0].agents.contains(&"antigravity".to_string()));
        // Hidden global-native agents stay excluded from the visibility list.
        assert!(!installed[0].agents.contains(&"dexto".to_string()));
        assert!(!installed[0].agents.contains(&"loaf".to_string()));

        // Project scope has no canonical dir — nothing listed.
        assert!(list_installed_skills(&env, false, &[]).is_empty());

        // After linking Antigravity's global dir, the skill becomes visible to it.
        std::fs::create_dir_all(tmp.path().join("home/.gemini/config")).unwrap();
        assert!(matches!(
            link_agent(
                crate::core::agents::get_agent("antigravity").unwrap(),
                true,
                &env,
                false
            ),
            LinkOutcome::Linked { .. }
        ));
        let installed = list_installed_skills(&env, true, &[]);
        assert!(installed[0].agents.contains(&"antigravity".to_string()));
    }

    #[test]
    fn list_installed_skills_reports_dir_linked_agents() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, false, &env);
        fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
        assert!(matches!(
            link_agent(
                crate::core::agents::get_agent("windsurf").unwrap(),
                false,
                &env,
                false
            ),
            LinkOutcome::Linked { .. }
        ));

        let installed = list_installed_skills(&env, false, &[]);
        assert_eq!(installed.len(), 1);
        assert!(installed[0].agents.contains(&"windsurf".to_string()));
    }

    #[test]
    fn scan_installed_lists_canonical_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, false, &env);

        let names = scan_installed(&env, false);
        assert_eq!(names, vec!["pdf".to_string()]);
    }

    #[test]
    fn move_skill_disables_then_enables_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, false, &env);

        // Disable: moves out of canonical, into disabled-skills.
        move_skill("pdf", false, false, &env).unwrap();
        assert!(!tmp.path().join(".agents/skills/pdf").exists());
        assert!(
            tmp.path()
                .join(".agents/disabled-skills/pdf/SKILL.md")
                .exists()
        );
        assert!(scan_installed(&env, false).is_empty());
        assert_eq!(scan_disabled(&env, false), vec!["pdf".to_string()]);

        // Enable: moves back.
        move_skill("pdf", false, true, &env).unwrap();
        assert!(tmp.path().join(".agents/skills/pdf/SKILL.md").exists());
        assert!(scan_disabled(&env, false).is_empty());
    }

    #[test]
    fn list_disabled_skills_reports_hidden_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, false, &env);
        move_skill("pdf", false, false, &env).unwrap();

        let disabled = list_disabled_skills(&env, false);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].name, "pdf");
        assert!(disabled[0].agents.is_empty());
        assert_eq!(disabled[0].scope, "project");
    }
}
