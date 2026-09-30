//! Unit tests for the manager's selection helpers.

use crate::core::install::ScannedSkill;
use crate::core::test_utils::env_at;
use crate::error::SkillsError;
use crate::manager::select::{SelectionOp, apply_selection, resolve_target_agents};

fn scanned(name: &str) -> ScannedSkill {
    ScannedSkill {
        name: name.to_string(),
        dir_name: name.to_string(),
    }
}

#[test]
fn selection_matches_on_reported_names_across_sources() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    // On-disk copies, so a Remove actually deletes something and reports it.
    for (base, rel) in [
        (".agents/skills", "pdf"),
        (".agents/skills", "PDF Master"),
        (".agents/disabled-skills", "Legacy Skill"),
    ] {
        let dir = tmp.path().join(base).join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "x").unwrap();
    }
    let installed = vec![scanned("pdf"), scanned("PDF Master")];
    let disabled = vec![ScannedSkill {
        name: "legacy-skill".to_string(),
        dir_name: "Legacy Skill".to_string(),
    }];
    let requested = vec![
        "pdf".to_string(),
        "legacy-skill".to_string(),
        "unknown".to_string(),
    ];

    // Only reported (frontmatter) names resolve; "unknown" is missing.
    let (applied, _already, missing) = apply_selection(
        SelectionOp::Remove,
        &requested,
        &[&installed, &disabled],
        &[],
        &env,
    )
    .unwrap();
    assert_eq!(applied, vec!["legacy-skill", "pdf"]);
    assert_eq!(missing, vec!["unknown".to_string()]);
}

#[test]
fn selection_classifies_already_and_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let installed = vec![scanned("pdf")];
    let disabled = vec![scanned("old")];

    // Disabling a disabled skill is already; an unknown name is missing.
    let (applied, already, missing) = apply_selection(
        SelectionOp::Disable,
        &["old".to_string(), "ghost".to_string()],
        &[&installed],
        &disabled,
        &env,
    )
    .unwrap();
    assert!(applied.is_empty());
    assert_eq!(already, vec!["old".to_string()]);
    assert_eq!(missing, vec!["ghost".to_string()]);
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
