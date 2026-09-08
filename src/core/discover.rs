//! SKILL.md discovery and frontmatter parsing.
//!
//! Priority container dirs (repo root + `skills/` + `.curated/.experimental/.system` +
//! each agent's project skills dir) recurse at most 3 levels, with shallow shadowing deep.
//! A root `SKILL.md` short-circuits the whole source to a single skill; when the
//! container walk finds nothing, full-tree recursion (max 5 levels) falls back.
//! Not supported: installed-project-skill filtering and plugin manifests.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Result, SkillsError};

/// Default max depth when searching within known container dirs.
pub const DEFAULT_SKILL_CONTAINER_DEPTH: usize = 3;

/// Dirs skipped during recursive search.
const SKIP_DIRS: [&str; 5] = ["node_modules", ".git", "dist", "build", "__pycache__"];

/// Each agent's project-level skills dir (one of the container dirs).
///
/// Kept by hand, intentionally independent of the agent table in `agents.rs`: this
/// list is the set of container dirs searched when discovering skills inside a
/// *source* repo, and it includes dirs that map to no registered agent (e.g.
/// `.github/skills`, `.codex/skills`). When an agent gains its own project dir,
/// decide whether discovery should cover it and update both lists.
pub const AGENT_PROJECT_SKILL_DIRS: [&str; 27] = [
    ".agents/skills",
    ".claude/skills",
    ".cline/skills",
    ".codebuddy/skills",
    ".codex/skills",
    ".commandcode/skills",
    ".continue/skills",
    ".github/skills",
    ".goose/skills",
    ".grok/skills",
    ".iflow/skills",
    ".junie/skills",
    ".kilocode/skills",
    ".kimchi/skills",
    ".kiro/skills",
    ".minimax/skills",
    ".mux/skills",
    ".neovate/skills",
    ".opencode/skills",
    ".openhands/skills",
    ".pi/skills",
    ".qoder/skills",
    ".roo/skills",
    ".trae/skills",
    ".windsurf/skills",
    ".zcode/skills",
    ".zencoder/skills",
];

/// A discovered skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name (from frontmatter).
    pub name: String,
    /// Skill description (from frontmatter).
    pub description: String,
    /// Directory containing SKILL.md.
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: Option<noyalib::Value>,
}

/// Split the `---`-delimited frontmatter, returning the YAML data.
/// Returns None when there is no frontmatter.
fn split_frontmatter(raw: &str) -> Option<&str> {
    let rest = raw
        .strip_prefix("---\r\n")
        .or_else(|| raw.strip_prefix("---\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// Parse a single SKILL.md; return None on any error (read failure / invalid YAML / missing fields).
/// Internal skills are hidden by default unless explicitly requested or `INSTALL_INTERNAL_SKILLS=1`.
pub fn parse_skill_md(skill_md: &Path) -> Option<Skill> {
    parse_skill_md_inner(skill_md, false)
}

/// Like [`parse_skill_md`], but allows including internal skills when `include_internal` is true.
pub fn parse_skill_md_inner(skill_md: &Path, include_internal: bool) -> Option<Skill> {
    let content = fs::read_to_string(skill_md).ok()?;
    let data = split_frontmatter(&content)?;
    let fm: Frontmatter = noyalib::from_str(data).ok()?;
    let name = fm.name?;
    let description = fm.description?;

    // internal skill: only visible when explicitly requested or INSTALL_INTERNAL_SKILLS=1.
    let is_internal = fm
        .metadata
        .as_ref()
        .and_then(|m| m.get("internal"))
        .and_then(|m| m.as_bool())
        .unwrap_or(false);
    if is_internal && !include_internal && !install_internal_skills() {
        return None;
    }

    Some(Skill {
        name,
        description,
        dir: skill_md.parent()?.to_path_buf(),
    })
}

fn install_internal_skills() -> bool {
    match std::env::var("INSTALL_INTERNAL_SKILLS") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => false,
    }
}

/// Validate that a subpath does not escape the base dir (path traversal guard).
pub fn is_subpath_safe(base: &Path, subpath: &str) -> bool {
    use std::path::Component;

    // Resolve the base to an absolute path (handles symlinks like /var -> /private/var).
    let base_abs = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    // Lexically walk `subpath` from the base, resolving `.` / `..` without touching
    // the filesystem. This is correct even when the target path does not exist yet:
    // `canonicalize` would fail there and we'd fall back to a raw path whose `..`
    // segments defeat component-wise prefix comparison.
    let mut target = base_abs.clone();
    for comp in Path::new(subpath).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Popping past the base root means the subpath escapes upward.
                if !target.pop() {
                    return false;
                }
            }
            Component::RootDir => return false, // absolute subpaths are not allowed
            Component::Prefix(_) | Component::Normal(_) => target.push(comp.as_os_str()),
        }
    }
    target == base_abs || target.starts_with(&base_abs)
}

/// Try to parse `dir/SKILL.md` and add it to results (shallow shadowing by name). Returns whether the dir has a SKILL.md.
fn try_add_skill_at(
    dir: &Path,
    include_internal: bool,
    seen: &mut HashSet<String>,
    skills: &mut Vec<Skill>,
) -> bool {
    if !dir.join("SKILL.md").is_file() {
        return false;
    }
    if let Some(skill) = parse_skill_md_inner(&dir.join("SKILL.md"), include_internal)
        && !seen.contains(&skill.name)
    {
        seen.insert(skill.name.clone());
        skills.push(skill);
    }
    true
}

/// Directed walk of container dirs: each level checks subdirs for SKILL.md; on a hit, don't descend further.
fn walk_skill_dirs(
    dir: &Path,
    max_depth: usize,
    depth: usize,
    include_internal: bool,
    seen: &mut HashSet<String>,
    skills: &mut Vec<Skill>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // Use fs::metadata (follows symlinks) instead of entry.metadata /
        // entry.file_type so symlinked skill dirs — e.g. goose's
        // `pdf -> ~/.skills-manager/skills/pdf` — are traversed, not skipped.
        let Ok(md) = fs::metadata(entry.path()) else {
            continue;
        };
        if !md.is_dir() {
            continue;
        }
        let child = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let found = try_add_skill_at(&child, include_internal, seen, skills);
        if found || depth >= max_depth {
            continue;
        }
        walk_skill_dirs(&child, max_depth, depth + 1, include_internal, seen, skills);
    }
}

/// Full-tree recursion fallback, max 5 levels, collecting SKILL.md dirs at each level.
fn find_all_skill_dirs(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    include_internal: bool,
    seen: &mut HashSet<String>,
    skills: &mut Vec<Skill>,
) {
    if depth > max_depth {
        return;
    }
    try_add_skill_at(dir, include_internal, seen, skills);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(md) = fs::metadata(entry.path()) else {
            continue;
        };
        if !md.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        find_all_skill_dirs(
            &entry.path(),
            depth + 1,
            max_depth,
            include_internal,
            seen,
            skills,
        );
    }
}

/// Discover skills within `base` (or a `subpath`-scoped range).
///
/// `include_internal`: include internal skills when explicitly specifying a skill (`--skill` or `@skill`).
pub fn discover_skills(
    base: &Path,
    subpath: Option<&str>,
    include_internal: bool,
) -> Result<Vec<Skill>> {
    if let Some(sp) = subpath
        && !is_subpath_safe(base, sp)
    {
        return Err(SkillsError::msg(format!(
            "Invalid subpath: \"{sp}\" resolves outside the repository directory. Subpath must not contain \"..\" segments that escape the base path."
        )));
    }
    let search_path = base.join(subpath.unwrap_or(""));
    let mut skills: Vec<Skill> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // A root SKILL.md hit short-circuits: the whole source is one skill.
    if search_path.join("SKILL.md").is_file()
        && let Some(skill) = parse_skill_md_inner(&search_path.join("SKILL.md"), include_internal)
        && !seen.contains(&skill.name)
    {
        seen.insert(skill.name.clone());
        skills.push(skill);
        return Ok(skills);
    }

    // Priority container dirs: repo root depth=1, other containers depth=3.
    let mut priority: Vec<PathBuf> = vec![search_path.clone()];
    for rel in [
        "skills",
        "skills/.curated",
        "skills/.experimental",
        "skills/.system",
    ] {
        priority.push(search_path.join(rel));
    }
    for rel in AGENT_PROJECT_SKILL_DIRS {
        priority.push(search_path.join(rel));
    }
    for (i, dir) in priority.iter().enumerate() {
        let max_depth = if i == 0 {
            1
        } else {
            DEFAULT_SKILL_CONTAINER_DEPTH
        };
        walk_skill_dirs(dir, max_depth, 1, include_internal, &mut seen, &mut skills);
    }

    // Nothing in the priority containers: full-tree recursion fallback.
    if skills.is_empty() {
        find_all_skill_dirs(&search_path, 0, 5, include_internal, &mut seen, &mut skills);
    }
    Ok(skills)
}

/// Filter by input names (case-insensitive, exact match on name or directory name).
pub fn filter_skills(skills: &[Skill], input_names: &[String]) -> Vec<Skill> {
    let normalized: Vec<String> = input_names.iter().map(|n| n.to_lowercase()).collect();
    skills
        .iter()
        .filter(|s| {
            let name = s.name.to_lowercase();
            let dir_name = s
                .dir
                .file_name()
                .map(|f| f.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            normalized.iter().any(|i| *i == name || *i == dir_name)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_utils::write_skill_md;

    #[test]
    fn parse_valid_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = write_skill_md(tmp.path(), "pdf", "pdf");
        let skill = parse_skill_md(&md).unwrap();
        assert_eq!(skill.name, "pdf");
        assert_eq!(skill.description, "does pdf");
        assert_eq!(skill.dir, tmp.path().join("pdf"));
    }

    #[test]
    fn parse_missing_required_field_is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = tmp.path().join("SKILL.md");
        fs::write(&md, "---\nname: only-name\n---\nbody").unwrap();
        assert!(parse_skill_md(&md).is_none());
    }

    #[test]
    fn parse_invalid_yaml_is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = tmp.path().join("SKILL.md");
        fs::write(&md, "---\nname: [unclosed\n---\nbody").unwrap();
        assert!(parse_skill_md(&md).is_none());
    }

    #[test]
    fn parse_without_frontmatter_is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = tmp.path().join("SKILL.md");
        fs::write(&md, "# just a heading\n").unwrap();
        assert!(parse_skill_md(&md).is_none());
    }

    #[test]
    fn parse_quoted_and_block_description() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = tmp.path().join("SKILL.md");
        fs::write(
            &md,
            "---\nname: \"pdf\"\ndescription: |\n  Multi line\n  description here\n---\nbody",
        )
        .unwrap();
        let skill = parse_skill_md(&md).unwrap();
        assert_eq!(skill.name, "pdf");
        assert!(skill.description.contains("Multi line"));
    }

    #[test]
    fn parse_internal_skill_hidden_by_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let md = tmp.path().join("SKILL.md");
        fs::write(
            &md,
            "---\nname: secret\ndescription: internal\nmetadata:\n  internal: true\n---\nbody",
        )
        .unwrap();
        assert!(parse_skill_md(&md).is_none());
        // Visible when explicitly requested.
        assert!(parse_skill_md_inner(&md, true).is_some());
    }

    #[test]
    fn root_skill_short_circuits() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), ".", "root");
        write_skill_md(tmp.path(), "skills/other", "other");
        let skills = discover_skills(tmp.path(), None, false).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "root");
    }

    #[test]
    fn container_walk_respects_depth_boundary() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skills/pdf", "pdf");
        write_skill_md(tmp.path(), "skills/category/pdf", "pdf-nested");
        // 4th-level dir under skills/, beyond the default container depth (3 levels).
        write_skill_md(tmp.path(), "skills/category/sub/x/pdf", "pdf-deep");
        let skills = discover_skills(tmp.path(), None, false).unwrap();
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pdf"));
        assert!(names.contains(&"pdf-nested"));
        assert!(!names.contains(&"pdf-deep"));
    }

    #[test]
    fn shallow_skill_shadows_deep_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skills/pdf", "pdf");
        write_skill_md(tmp.path(), "skills/pdf/pdf", "pdf");
        let skills = discover_skills(tmp.path(), None, false).unwrap();
        assert_eq!(skills.iter().filter(|s| s.name == "pdf").count(), 1);
        assert_eq!(skills[0].dir, tmp.path().join("skills/pdf"));
    }

    #[test]
    fn skip_dirs_are_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skills/pdf", "pdf");
        write_skill_md(tmp.path(), "node_modules/x", "x");
        write_skill_md(tmp.path(), "skills/.git/x", "git-x");
        let skills = discover_skills(tmp.path(), None, false).unwrap();
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pdf"));
        assert!(!names.contains(&"x"));
        assert!(!names.contains(&"git-x"));
    }

    #[test]
    fn unsafe_subpath_is_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "pdf", "pdf");
        assert!(discover_skills(tmp.path(), Some("../evil"), false).is_err());
    }

    #[test]
    fn filter_matches_name_or_dir_case_insensitive() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skills/pdf", "PDF Master");
        write_skill_md(tmp.path(), "skills/doc", "docx");
        let skills = discover_skills(tmp.path(), None, false).unwrap();
        let hit = filter_skills(&skills, &["pdf master".to_string()]);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].name, "PDF Master");
        let hit2 = filter_skills(&skills, &["pdf".to_string()]);
        assert_eq!(hit2.len(), 1);
    }
}
