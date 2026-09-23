//! Unit tests for the linking machinery.

use std::fs;
use std::path::Path;

use crate::core::agents::get_agent;
use crate::core::install::{
    install_skill, list_installed_skills, move_skill, sanitize_name, scan_installed,
};
use crate::core::link::outcome::LinkOutcome;
use crate::core::link::{is_agent_linked, link_agent, private_content, unlink_agent};
use crate::core::test_utils::{env_at, skill_frontmatter, write_and_parse_skill};

/// Sorted copy of a names vec (read_dir order is arbitrary).
fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn link_agent_creates_relative_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
    let agent = get_agent("cursor").unwrap();

    let outcome = link_agent(agent, &env);
    assert!(
        matches!(
            outcome,
            LinkOutcome::Linked {
                ref adopted,
                ref quarantined,
                ref conflicts,
            } if adopted.is_empty() && quarantined.is_empty() && conflicts.is_empty()
        ),
        "got {outcome:?}"
    );
    let link = tmp.path().join(".cursor/skills");
    assert!(link.is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("../.agents/skills")
    );
}

#[test]
fn link_agent_skips_when_agent_root_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("cursor").unwrap(); // ~/.cursor does not exist

    assert!(matches!(link_agent(agent, &env), LinkOutcome::Skipped));
    assert!(!tmp.path().join(".cursor").exists());
}

#[test]
fn link_agent_claude_code_links_even_without_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("claude-code").unwrap(); // ~/.claude does not exist

    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_native_agent_is_already_linked() {
    // Cline's skills dir is ~/.agents/skills itself — nothing to link.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let cline = get_agent("cline").unwrap();
    assert!(matches!(
        link_agent(cline, &env),
        LinkOutcome::AlreadyLinked
    ));
    assert!(is_agent_linked(cline, &env));
    assert!(matches!(unlink_agent(cline, &env), LinkOutcome::NotLinked));
}

#[test]
fn link_agent_skipped_when_dir_cannot_be_resolved() {
    // promptscript's skills dir is env-var based and the var is unset.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("promptscript").unwrap();
    assert!(matches!(link_agent(agent, &env), LinkOutcome::Skipped));
}

#[test]
fn link_agent_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
    let agent = get_agent("cursor").unwrap();

    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));
    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::AlreadyLinked
    ));
}

#[test]
fn link_agent_refuses_foreign_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
    fs::create_dir_all(tmp.path().join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("elsewhere"),
        tmp.path().join(".cursor/skills"),
    )
    .unwrap();
    let agent = get_agent("cursor").unwrap();

    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Refused { .. }
    ));
}

#[test]
fn link_agent_adopts_existing_skills_and_links() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    // A symlinked skill pointing elsewhere (e.g. into a skills hub) is moved
    // as a link, preserving its target.
    fs::create_dir_all(tmp.path().join("hub/hub-skill")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("hub/hub-skill"),
        tmp.path().join(".claude/skills/hub-skill"),
    )
    .unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted,
            quarantined,
            conflicts,
        } => {
            assert_eq!(sorted(adopted), vec!["hub-skill", "my-skill"]);
            assert!(quarantined.is_empty());
            assert!(conflicts.is_empty());
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    let moved_link = tmp.path().join(".agents/skills/hub-skill");
    assert!(moved_link.is_symlink());
    assert_eq!(
        fs::read_link(&moved_link).unwrap(),
        tmp.path().join("hub/hub-skill")
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());
    // Nothing was quarantined, so no `.misc` dir exists.
    assert!(!tmp.path().join(".agents/skills/.misc").exists());
}

#[test]
fn link_agent_quarantines_non_skill_entries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    fs::write(
        tmp.path().join(".claude/skills/my-skill/SKILL.md"),
        skill_frontmatter("my-skill"),
    )
    .unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted,
            quarantined,
            conflicts,
        } => {
            assert_eq!(adopted, vec!["my-skill"]);
            assert_eq!(quarantined, vec!["README.txt"]);
            assert!(conflicts.is_empty());
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    assert!(
        tmp.path()
            .join(".agents/skills/.misc/claude-code/README.txt")
            .exists()
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());

    // The quarantine dot-dir never shows up as an installed skill.
    assert_eq!(scan_installed(&env), vec!["my-skill".to_string()]);
    let listed: Vec<String> = list_installed_skills(&env)
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(listed, vec!["my-skill".to_string()]);
}

#[test]
fn link_agent_drops_conflict_across_name_normalization() {
    // The canonical dir holds the sanitized name (`add` normalizes its target),
    // while the agent dir holds an unnormalized one — same skill, so it clashes.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".agents/skills/pdf-master")).unwrap();
    fs::write(
        tmp.path().join(".agents/skills/pdf-master/SKILL.md"),
        "canonical",
    )
    .unwrap();
    let existing = tmp.path().join(".claude/skills/PDF Master");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted, conflicts, ..
        } => {
            assert!(adopted.is_empty());
            assert_eq!(conflicts, vec!["PDF Master"]);
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    // Only the canonical copy survives.
    assert_eq!(
        fs::read_to_string(tmp.path().join(".agents/skills/pdf-master/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(!tmp.path().join(".agents/skills/PDF Master").exists());
    assert_eq!(scan_installed(&env), vec!["pdf-master".to_string()]);
}

#[test]
fn link_agent_keeps_only_one_of_two_sibling_names() {
    // Two entries in the same agent dir that normalize to the same name cannot
    // both be adopted; read_dir order decides which one wins.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    for dir in [".claude/skills/PDF Master", ".claude/skills/pdf-master"] {
        fs::create_dir_all(tmp.path().join(dir)).unwrap();
        fs::write(tmp.path().join(dir).join("SKILL.md"), "x").unwrap();
    }
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted, conflicts, ..
        } => {
            assert_eq!(adopted.len(), 1);
            assert_eq!(conflicts.len(), 1);
            assert_eq!(sanitize_name(&adopted[0]), sanitize_name(&conflicts[0]));
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert_eq!(scan_installed(&env).len(), 1);
}

#[test]
fn link_agent_drops_name_conflicts_in_favour_of_canonical() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    // Same name already installed in canonical; the canonical copy wins.
    fs::create_dir_all(tmp.path().join(".agents/skills/pdf")).unwrap();
    fs::write(tmp.path().join(".agents/skills/pdf/SKILL.md"), "canonical").unwrap();
    let existing = tmp.path().join(".claude/skills/pdf");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    // Another skill that does not clash is still adopted.
    fs::create_dir_all(tmp.path().join(".claude/skills/notes")).unwrap();
    fs::write(tmp.path().join(".claude/skills/notes/SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted,
            quarantined,
            conflicts,
        } => {
            assert_eq!(adopted, vec!["notes"]);
            assert!(quarantined.is_empty());
            assert_eq!(conflicts, vec!["pdf"]);
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    // Canonical copy untouched; the agent-side duplicate is gone.
    assert_eq!(
        fs::read_to_string(tmp.path().join(".agents/skills/pdf/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(tmp.path().join(".agents/skills/notes/SKILL.md").exists());
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_drops_conflicts_with_disabled_skills() {
    // A skill disabled into the disabled dir must not be re-imported when
    // linking an agent that holds its own copy of the same name.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let src = tmp.path().join("pdf");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, &env);
    move_skill("pdf", false, &env).unwrap();
    // The agent holds its own copy of the disabled skill plus a fresh one.
    let existing = tmp.path().join(".claude/skills/pdf");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    fs::create_dir_all(tmp.path().join(".claude/skills/notes")).unwrap();
    fs::write(tmp.path().join(".claude/skills/notes/SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted, conflicts, ..
        } => {
            assert_eq!(adopted, vec!["notes"]);
            assert_eq!(conflicts, vec!["pdf"]);
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    // The disabled copy stays disabled; the agent copy is dropped.
    assert!(!tmp.path().join(".agents/skills/pdf").exists());
    assert!(
        tmp.path()
            .join(".agents/disabled-skills/pdf/SKILL.md")
            .exists()
    );
    // The fresh skill was adopted normally.
    assert!(tmp.path().join(".agents/skills/notes/SKILL.md").exists());
}

#[test]
fn link_agent_drops_legacy_per_skill_links() {
    // Old-model agent dirs hold per-skill symlinks into the canonical dir. Their
    // content already lives there, so the links are dropped — moving them in
    // would create a self-referential symlink.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let src = tmp.path().join("pdf");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, &env);
    fs::create_dir_all(tmp.path().join(".cursor/skills")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join(".agents/skills/pdf"),
        tmp.path().join(".cursor/skills/pdf"),
    )
    .unwrap();
    let agent = get_agent("cursor").unwrap();

    match link_agent(agent, &env) {
        LinkOutcome::Linked {
            adopted, conflicts, ..
        } => {
            assert!(adopted.is_empty());
            assert_eq!(conflicts, vec!["pdf"]);
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    // The canonical skill is still a real dir, not a self-referential symlink.
    let canonical_skill = tmp.path().join(".agents/skills/pdf");
    assert!(canonical_skill.is_dir());
    assert!(!canonical_skill.is_symlink());
    assert!(canonical_skill.join("SKILL.md").exists());
    assert!(tmp.path().join(".cursor/skills").is_symlink());
}

#[test]
fn unlink_agent_does_not_restore_adopted_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));

    assert!(matches!(unlink_agent(agent, &env), LinkOutcome::Unlinked));
    // The adopted skill stays in the canonical dir (managed by `remove`).
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    let dir = tmp.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert!(fs::read_dir(&dir).unwrap().count() == 0);
}

#[test]
fn unlink_agent_removes_link_and_recreates_empty_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
    let agent = get_agent("cursor").unwrap();
    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));

    assert!(matches!(unlink_agent(agent, &env), LinkOutcome::Unlinked));
    let dir = tmp.path().join(".cursor/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert!(fs::read_dir(&dir).unwrap().count() == 0);
}

#[test]
fn unlink_agent_leaves_real_dirs_and_foreign_links_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(unlink_agent(agent, &env), LinkOutcome::NotLinked));
    assert!(tmp.path().join(".claude/skills/my-skill").exists());

    // Native agents never link.
    let uni = get_agent("cline").unwrap();
    assert!(matches!(unlink_agent(uni, &env), LinkOutcome::NotLinked));
}

#[test]
fn is_agent_linked_reflects_dir_links() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("cursor").unwrap();
    assert!(!is_agent_linked(agent, &env));
    fs::create_dir_all(tmp.path().join(".cursor")).unwrap();
    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));
    assert!(is_agent_linked(agent, &env));
    // Native agents are always "linked" (canonical is their dir).
    assert!(is_agent_linked(get_agent("cline").unwrap(), &env));
}

#[test]
fn private_content_classifies_skills_and_others() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    fs::write(tmp.path().join(".claude/skills/my-skill/SKILL.md"), "x").unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    let (skills, others) = private_content(agent, &env);
    assert_eq!(skills, vec!["my-skill"]);
    assert_eq!(others, vec!["README.txt"]);

    // A linked (or foreign-symlink) skills dir is not private content.
    assert!(matches!(
        link_agent(agent, &env),
        LinkOutcome::Linked { .. }
    ));
    assert_eq!(private_content(agent, &env), (Vec::new(), Vec::new()));
}
