//! Unit tests for the manager's selection helpers (moved verbatim from the bottom
//! of the original `manager.rs`).

use crate::core::source::parse_source;
use crate::core::test_utils::env_at;
use crate::error::SkillsError;
use crate::manager::select::{resolve_target_agents, resolve_to_remove, skill_filters};
use crate::manager::{
    AddRequest, missing_skill_error, rate_limit_hint, require_subpath, uses_github_api,
};

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
    let requested = vec![
        "pdf".to_string(),
        "legacy".to_string(),
        "unknown".to_string(),
    ];

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

#[test]
fn require_subpath_names_the_missing_path_and_the_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("skills/pdf")).unwrap();
    let source = parse_source("acme/skills/skills/pdf").unwrap();

    assert!(require_subpath(tmp.path(), "skills/pdf", &source).is_ok());

    // A missing subpath must be named instead of surfacing as a vague "no valid
    // skills found" (and only after a pointless whole-repo download).
    let msg = require_subpath(tmp.path(), "skills/doc", &source)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("skills/doc"), "{msg}");
    assert!(msg.contains("https://github.com/acme/skills"), "{msg}");
}

#[test]
fn uses_github_api_only_for_narrowed_github_requests() {
    // Anything the API cannot narrow is served by the whole-repo archive: a
    // repository-wide install, `--list`, and GitLab.
    let cases = [
        ("acme/skills", false, false),
        ("acme/skills", true, false),
        ("acme/skills/skills/pdf", false, true),
        ("acme/skills/skills/pdf", true, true),
        ("acme/skills@pdf", false, true),
        // `--list` needs the whole tree to report every skill, so a name cannot narrow it.
        ("acme/skills@pdf", true, false),
    ];
    for (source, list_only, expected) in cases {
        let parsed = parse_source(source).unwrap();
        assert_eq!(
            uses_github_api(&parsed, list_only),
            expected,
            "{source} (list_only={list_only})"
        );
    }

    // GitLab has no API path at all.
    let gitlab = parse_source("https://gitlab.com/acme/skills/-/tree/main/skills/pdf").unwrap();
    assert!(!uses_github_api(&gitlab, false));
    assert!(!uses_github_api(&gitlab, true));
}

#[test]
fn missing_skill_error_points_at_list() {
    let parsed = parse_source("acme/skills@pdf").unwrap();
    let req = AddRequest::new("acme/skills@pdf");

    let msg = missing_skill_error(&parsed, &req, "pdf").to_string();
    assert!(msg.contains("\"pdf\""), "{msg}");
    assert!(msg.contains("https://github.com/acme/skills"), "{msg}");
    assert!(msg.contains("--list"), "{msg}");
}

#[test]
fn rate_limit_hint_only_fires_for_403_and_429() {
    let limited = SkillsError::Http(Box::new(ureq::Error::StatusCode(403)));
    assert!(rate_limit_hint(&limited).contains("GITHUB_TOKEN"));
    assert!(
        rate_limit_hint(&SkillsError::Http(Box::new(ureq::Error::StatusCode(429))))
            .contains("GITHUB_TOKEN")
    );

    // A 5xx or a plain message says nothing about rate limits.
    let unavailable = SkillsError::Http(Box::new(ureq::Error::StatusCode(503)));
    assert!(rate_limit_hint(&unavailable).is_empty());
    assert!(rate_limit_hint(&SkillsError::msg("boom")).is_empty());
}
