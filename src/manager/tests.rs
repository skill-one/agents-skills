//! Unit tests for the manager's selection helpers (moved verbatim from the bottom
//! of the original `manager.rs`).

use crate::core::test_utils::env_at;
use crate::error::SkillsError;
use crate::manager::select::{resolve_target_agents, resolve_to_remove, skill_filters};

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
fn resolve_to_remove_matches_on_disk_names() {
    let installed = vec!["pdf".to_string()];
    let disabled = vec!["legacy".to_string()];
    let requested = vec!["pdf".to_string(), "legacy".to_string(), "unknown".to_string()];

    // Only on-disk dir names resolve; "unknown" matches nothing.
    assert_eq!(
        resolve_to_remove(&requested, &installed, &disabled),
        vec!["legacy", "pdf"]
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
