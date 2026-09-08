//! Unit tests for the manager's selection helpers (moved verbatim from the bottom
//! of the original `manager.rs`).

use crate::core::discover::Skill;
use crate::core::test_utils::{env_at, write_and_parse_skill};
use crate::error::SkillsError;
use crate::manager::select::{
    find_skill, matches_skill, resolve_target_agents, resolve_to_remove, skill_filters,
};

fn skills_with_dirs(pairs: &[(&str, &str)]) -> Vec<Skill> {
    pairs
        .iter()
        .map(|(dir, name)| {
            let mut s = write_and_parse_skill(std::path::Path::new(dir), name);
            // write_and_parse_skill derives dir from the SKILL.md path; use the skill dir.
            s.dir = std::path::PathBuf::from(dir);
            s
        })
        .collect()
}

#[test]
fn find_skill_prefers_name_then_skill_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let a = tmp.path().join("dir-a");
    let b = tmp.path().join("dir-b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let skills = skills_with_dirs(&[
        (a.to_str().unwrap(), "alpha"),
        (b.to_str().unwrap(), "beta"),
    ]);

    // By (sanitized) name.
    assert_eq!(find_skill(&skills, "Alpha", None).unwrap().name, "alpha");
    // By skillPath directory name.
    assert_eq!(
        find_skill(&skills, "missing", Some("x/dir-b"))
            .unwrap()
            .name,
        "beta"
    );
    // Ambiguous without a match.
    assert!(find_skill(&skills, "missing", None).is_none());
}

#[test]
fn matches_skill_filters_case_insensitively() {
    let filter = vec!["PDF".to_string()];
    assert!(matches_skill("pdf", &filter));
    assert!(!matches_skill("git", &filter));
    assert!(matches_skill("anything", &[]));
}

#[test]
fn skill_filters_merges_args_and_at_filter() {
    assert_eq!(skill_filters(&[], Some("pdf")), vec!["pdf"]);
    assert_eq!(skill_filters(&["x".to_string()], None), vec!["x"]);
    assert_eq!(
        skill_filters(&["x".to_string()], Some("pdf")),
        vec!["x", "pdf"]
    );
    assert!(skill_filters(&[], None).is_empty());
}

#[test]
fn resolve_to_remove_prefers_lock_keys() {
    let installed = vec!["PDF".to_string()];
    let lock_keys = vec!["pdf".to_string()];
    let requested = vec!["pdf".to_string(), "unknown".to_string()];

    // Lock keys take priority: "pdf" (not the on-disk "PDF" casing).
    assert_eq!(
        resolve_to_remove(&requested, &installed, &[], &lock_keys),
        vec!["pdf"]
    );
}

#[test]
fn resolve_to_remove_matches_disabled_without_lock() {
    let installed = Vec::new();
    let disabled = vec!["legacy".to_string()];
    let lock_keys = Vec::new();
    let requested = vec!["legacy".to_string()];

    // A disabled skill with no lockfile entry is still resolvable for removal.
    assert_eq!(
        resolve_to_remove(&requested, &installed, &disabled, &lock_keys),
        vec!["legacy"]
    );
}

#[test]
fn resolve_target_agents_validates_names() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    assert!(resolve_target_agents(&["claude-code".to_string()], &env).is_ok());
    assert!(matches!(
        resolve_target_agents(&["nope".to_string()], &env),
        Err(SkillsError::InvalidAgents(_))
    ));
}
