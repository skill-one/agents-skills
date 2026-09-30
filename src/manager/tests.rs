//! Unit tests for the manager's selection helpers.

use crate::core::install::ScannedSkill;
use crate::core::test_utils::env_at;
use crate::error::SkillsError;
use crate::manager::select::{resolve_target_agents, resolve_to_remove};

fn scanned(name: &str) -> ScannedSkill {
    ScannedSkill {
        name: name.to_string(),
        dir_name: name.to_string(),
    }
}

#[test]
fn resolve_to_remove_matches_on_reported_names() {
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

    // Only reported (frontmatter) names resolve; "unknown" matches nothing.
    let selected = resolve_to_remove(&requested, &installed, &disabled);
    let names: Vec<&str> = selected.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["legacy-skill", "pdf"]);

    // The resolved entry carries the real on-disk directory.
    let legacy = selected.iter().find(|s| s.name == "legacy-skill").unwrap();
    assert_eq!(legacy.dir_name, "Legacy Skill");
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
