//! Unit tests for the linking machinery (moved verbatim from the bottom of the
//! original `link.rs`).

use std::fs;
use std::path::Path;

use crate::core::agents::{Env, get_agent};
use crate::core::install::{install_skill, move_skill};
use crate::core::link::outcome::LinkOutcome;
use crate::core::link::{
    is_agent_linked, link_agent, pending_backup, private_content, unlink_agent,
};
use crate::core::test_utils::{env_at, write_and_parse_skill};

/// Env with distinct home/cwd (for global-scope tests).
fn split_env(tmp: &tempfile::TempDir) -> Env {
    Env::new(
        tmp.path().join("home"),
        tmp.path().join("config"),
        tmp.path().join("project"),
    )
}

/// Sorted copy of a names vec (read_dir order is arbitrary).
fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn link_agent_creates_relative_symlink_project() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
    let agent = get_agent("windsurf").unwrap();

    let outcome = link_agent(agent, false, &env, false);
    assert!(
        matches!(
            outcome,
            LinkOutcome::Linked {
                backup_dir: None,
                ..
            }
        ),
        "got {outcome:?}"
    );
    let link = tmp.path().join(".windsurf/skills");
    assert!(link.is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("../.agents/skills")
    );
}

#[test]
fn link_agent_global_links_home_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = split_env(&tmp);
    fs::create_dir_all(env.home.join(".claude/skills")).unwrap();
    let agent = get_agent("claude-code").unwrap();

    let outcome = link_agent(agent, true, &env, false);
    assert!(matches!(outcome, LinkOutcome::Linked { .. }));
    let link = env.home.join(".claude/skills");
    assert!(link.is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("../.agents/skills")
    );
}

#[test]
fn link_agent_global_links_project_universal_agent_with_vendor_dir() {
    // Antigravity is universal at project scope but reads the vendor-specific
    // ~/.gemini/config/skills dir globally — global scope needs a real link.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = split_env(&tmp);
    // Global gate: the parent of the agent skills dir must exist.
    fs::create_dir_all(env.home.join(".gemini/config")).unwrap();
    let agent = get_agent("antigravity").unwrap();

    let outcome = link_agent(agent, true, &env, false);
    assert!(
        matches!(
            outcome,
            LinkOutcome::Linked {
                backup_dir: None,
                ..
            }
        ),
        "got {outcome:?}"
    );
    let link = env.home.join(".gemini/config/skills");
    assert!(link.is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("../../.agents/skills")
    );
    assert!(is_agent_linked(agent, true, &env));

    // A globally installed skill is visible through the link.
    let src = tmp.path().join("src-skill");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, true, &env);
    assert!(link.join("pdf/SKILL.md").exists());

    // Unlink restores an empty real dir and disconnects cleanly.
    assert!(matches!(
        unlink_agent(agent, true, &env),
        LinkOutcome::Unlinked { .. }
    ));
    assert!(!link.is_symlink());
    assert!(link.is_dir());
    assert!(!is_agent_linked(agent, true, &env));
}

#[test]
fn link_agent_global_skipped_when_global_root_missing() {
    // Antigravity installed (marker ~/.gemini/antigravity) but the shared config
    // root ~/.gemini/config absent: do not fabricate it — Skipped.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = split_env(&tmp);
    fs::create_dir_all(env.home.join(".gemini/antigravity")).unwrap();
    let agent = get_agent("antigravity").unwrap();

    assert!(matches!(
        link_agent(agent, true, &env, false),
        LinkOutcome::Skipped
    ));
    assert!(!env.home.join(".gemini/config/skills").exists());
}

#[test]
fn link_agent_global_native_agent_is_already_linked() {
    // Cline's global dir is ~/.agents/skills itself — native globally too.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = split_env(&tmp);
    let cline = get_agent("cline").unwrap();
    assert!(matches!(
        link_agent(cline, true, &env, false),
        LinkOutcome::AlreadyLinked
    ));
    assert!(is_agent_linked(cline, true, &env));
    assert!(matches!(
        unlink_agent(cline, true, &env),
        LinkOutcome::NotLinked
    ));
}

#[test]
fn link_agent_global_parks_existing_vendor_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = split_env(&tmp);
    let existing = env.home.join(".gemini/config/skills/old-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("antigravity").unwrap();

    match link_agent(agent, true, &env, false) {
        LinkOutcome::Linked { .. } => {}
        other => panic!("expected Linked, got {other:?}"),
    }
    let slot = env.home.join(".agents/backup-skills/antigravity");
    assert!(slot.join("skills/old-skill/SKILL.md").exists());
    assert!(env.home.join(".gemini/config/skills").is_symlink());

    // Unlink restores the parked dir losslessly.
    assert!(matches!(
        unlink_agent(agent, true, &env),
        LinkOutcome::Unlinked { .. }
    ));
    let restored = env.home.join(".gemini/config/skills");
    assert!(restored.is_dir());
    assert!(!restored.is_symlink());
    assert!(restored.join("old-skill/SKILL.md").exists());
    assert!(!slot.exists());
}

#[test]
fn link_agent_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
    let agent = get_agent("windsurf").unwrap();

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::AlreadyLinked
    ));
}

#[test]
fn link_agent_refuses_foreign_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
    fs::create_dir_all(tmp.path().join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("elsewhere"),
        tmp.path().join(".windsurf/skills"),
    )
    .unwrap();
    let agent = get_agent("windsurf").unwrap();

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Refused { .. }
    ));
}

#[test]
fn link_agent_skips_when_agent_root_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("windsurf").unwrap(); // .windsurf does not exist

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Skipped
    ));
    assert!(!tmp.path().join(".windsurf").exists());
}

#[test]
fn link_agent_claude_code_links_even_without_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("claude-code").unwrap(); // .claude does not exist

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_parks_existing_content_and_links() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    // A symlinked skill pointing elsewhere (e.g. into a skills hub) is parked as is.
    fs::create_dir_all(tmp.path().join("hub/other-skill")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("hub/other-skill"),
        tmp.path().join(".claude/skills/other-skill"),
    )
    .unwrap();
    // A stray file is parked too.
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, false) {
        LinkOutcome::Linked {
            parked_skills,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(sorted(parked_skills), vec!["my-skill", "other-skill"]);
            assert_eq!(parked_others, vec!["README.txt"]);
            assert_eq!(
                backup_dir,
                Some(tmp.path().join(".agents/backup-skills/claude-code/skills"))
            );
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    // The whole dir was parked as is; the agent dir is linked.
    assert!(tmp.path().join(".claude/skills").is_symlink());
    let parked = tmp.path().join(".agents/backup-skills/claude-code/skills");
    assert!(parked.join("my-skill/SKILL.md").exists());
    assert!(parked.join("other-skill").is_symlink());
    assert!(parked.join("README.txt").exists());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/manifest.json")
            .exists()
    );
    // Skills are NOT in the canonical dir (plain link only parks).
    assert!(!tmp.path().join(".agents/skills").exists());
}

#[test]
fn link_agent_parks_dir_with_only_stray_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills")).unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, false) {
        LinkOutcome::Linked {
            parked_skills,
            parked_others,
            ..
        } => {
            assert!(parked_skills.is_empty());
            assert_eq!(parked_others, vec!["README.txt"]);
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert!(tmp.path().join(".claude/skills").is_symlink());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/README.txt")
            .exists()
    );
}

#[test]
fn link_agent_refuses_stale_backup_slot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/old")).unwrap();
    // A previous park left content behind.
    let parked = tmp.path().join(".agents/backup-skills/claude-code/skills");
    fs::create_dir_all(&parked).unwrap();
    fs::write(parked.join("parked.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, false) {
        LinkOutcome::Refused { reason } => {
            assert!(reason.contains("backup"), "reason: {reason}");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    // Nothing was touched.
    assert!(tmp.path().join(".claude/skills/old").exists());
    assert!(!tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_degenerate_empty_parked_dir_does_not_block() {
    // A leftover empty parked dir is not a backup; parking replaces it.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    fs::create_dir_all(tmp.path().join(".agents/backup-skills/claude-code/skills")).unwrap();
    let agent = get_agent("claude-code").unwrap();

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert!(tmp.path().join(".claude/skills").is_symlink());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/my-skill/SKILL.md")
            .exists()
    );
}

#[test]
fn link_agent_migrate_moves_content_and_links() {
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

    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated {
            moved,
            skipped,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(sorted(moved), vec!["hub-skill", "my-skill"]);
            assert!(skipped.is_empty());
            assert!(parked_others.is_empty());
            assert!(backup_dir.is_none(), "nothing parked, no slot needed");
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    let moved_link = tmp.path().join(".agents/skills/hub-skill");
    assert!(moved_link.is_symlink());
    assert_eq!(
        fs::read_link(&moved_link).unwrap(),
        tmp.path().join("hub/hub-skill")
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());
    assert!(
        !tmp.path()
            .join(".agents/backup-skills/claude-code")
            .exists()
    );
}

#[test]
fn link_agent_migrate_skips_same_name_parking_agent_copy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/pdf");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    // Another skill that does not clash is still migrated.
    fs::create_dir_all(tmp.path().join(".claude/skills/notes")).unwrap();
    fs::write(tmp.path().join(".claude/skills/notes/SKILL.md"), "x").unwrap();
    // Same name already installed in canonical; the canonical copy wins.
    fs::create_dir_all(tmp.path().join(".agents/skills/pdf")).unwrap();
    fs::write(tmp.path().join(".agents/skills/pdf/SKILL.md"), "canonical").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated {
            moved,
            skipped,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(moved, vec!["notes"]);
            assert_eq!(skipped, vec!["pdf"]);
            assert!(parked_others.is_empty());
            assert_eq!(
                backup_dir,
                Some(tmp.path().join(".agents/backup-skills/claude-code/skills"))
            );
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    // Canonical copy untouched; the agent-side copy is parked for restore.
    assert_eq!(
        fs::read_to_string(tmp.path().join(".agents/skills/pdf/SKILL.md")).unwrap(),
        "canonical"
    );
    assert_eq!(
        fs::read_to_string(
            tmp.path()
                .join(".agents/backup-skills/claude-code/skills/pdf/SKILL.md")
        )
        .unwrap(),
        "agent copy"
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_migrate_keeps_disabled_skills_disabled() {
    // A skill disabled into the disabled dir must not be re-imported when
    // linking an agent that holds its own copy of the same skill.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let src = tmp.path().join("src-skill");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, false, &env);
    move_skill("pdf", false, false, &env).unwrap();
    // The agent holds its own copy of the disabled skill plus a fresh one.
    let existing = tmp.path().join(".claude/skills/pdf");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    fs::create_dir_all(tmp.path().join(".claude/skills/notes")).unwrap();
    fs::write(tmp.path().join(".claude/skills/notes/SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated { moved, skipped, .. } => {
            assert_eq!(moved, vec!["notes"]);
            assert_eq!(skipped, vec!["pdf"]);
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    // The disabled copy stays disabled; the agent copy stays parked.
    assert!(!tmp.path().join(".agents/skills/pdf").exists());
    assert!(
        tmp.path()
            .join(".agents/disabled-skills/pdf/SKILL.md")
            .exists()
    );
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/pdf/SKILL.md")
            .exists()
    );
    // The fresh skill was migrated normally.
    assert!(tmp.path().join(".agents/skills/notes/SKILL.md").exists());
}

#[test]
fn migrate_from_backup_keeps_disabled_skills_disabled() {
    // Already linked agent with a parked copy; the canonical skill is then
    // disabled. Rerunning with --migrate must not pull the parked copy in.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    // Canonical skill + the agent's own copy of the same name.
    let src = tmp.path().join("src-skill");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, false, &env);
    let existing = tmp.path().join(".claude/skills/pdf");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "agent copy").unwrap();
    let agent = get_agent("claude-code").unwrap();
    // Plain link: the agent copy is parked (name clash keeps canonical).
    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated { moved, skipped, .. } => {
            assert!(moved.is_empty());
            assert_eq!(skipped, vec!["pdf"]);
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    // Disable the canonical copy, then rerun --migrate.
    move_skill("pdf", false, false, &env).unwrap();
    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated { moved, skipped, .. } => {
            assert!(moved.is_empty(), "disabled skill must not be imported");
            assert_eq!(skipped, vec!["pdf"]);
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    assert!(!tmp.path().join(".agents/skills/pdf").exists());
    assert!(
        tmp.path()
            .join(".agents/disabled-skills/pdf/SKILL.md")
            .exists()
    );
}

#[test]
fn link_agent_migrate_parks_stray_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    fs::write(tmp.path().join(".claude/skills/my-skill/SKILL.md"), "x").unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated {
            moved,
            skipped: _,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(moved, vec!["my-skill"]);
            assert_eq!(parked_others, vec!["README.txt"]);
            assert_eq!(
                backup_dir,
                Some(tmp.path().join(".agents/backup-skills/claude-code/skills"))
            );
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/README.txt")
            .exists()
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_migrate_pulls_parked_skills_from_backup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    // Plain link first: the skill is parked, not migrated.
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert!(!tmp.path().join(".agents/skills").exists());

    // Rerunning with --migrate on the already linked agent moves the parked
    // skill into the canonical dir; the stray file stays parked.
    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated {
            moved,
            skipped,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(moved, vec!["my-skill"]);
            assert!(skipped.is_empty());
            assert_eq!(parked_others, vec!["README.txt"]);
            assert!(backup_dir.is_some());
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/README.txt")
            .exists()
    );
    assert!(tmp.path().join(".claude/skills").is_symlink());
}

#[test]
fn link_agent_migrate_already_linked_without_backup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    // --migrate on a linked agent with an empty slot stays AlreadyLinked.
    assert!(matches!(
        link_agent(agent, false, &env, true),
        LinkOutcome::AlreadyLinked
    ));
}

#[test]
fn link_agent_parks_legacy_per_skill_links() {
    // Old-model agent dirs (per-skill links into canonical) are parked whole
    // and restored as is — nothing is dropped.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    // Canonical already holds the skill (old model wrote it there).
    let src = tmp.path().join("src-skill");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, false, &env);
    // Old-model agent dir: per-skill symlink pointing into canonical.
    fs::create_dir_all(tmp.path().join(".windsurf/skills")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join(".agents/skills/pdf"),
        tmp.path().join(".windsurf/skills/pdf"),
    )
    .unwrap();
    let agent = get_agent("windsurf").unwrap();

    match link_agent(agent, false, &env, false) {
        LinkOutcome::Linked { backup_dir, .. } => {
            assert!(
                backup_dir.is_some(),
                "legacy links are parked, not taken over"
            );
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    let link = tmp.path().join(".windsurf/skills");
    assert!(link.is_symlink());
    assert!(link.join("pdf/SKILL.md").exists());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/windsurf/skills/pdf")
            .is_symlink()
    );
}

#[test]
fn link_agent_migrate_keeps_legacy_links_parked() {
    // Legacy per-skill links already point into the canonical dir: --migrate
    // leaves them parked instead of moving them into canonical.
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let src = tmp.path().join("src-skill");
    let skill = write_and_parse_skill(&src, "pdf");
    install_skill(&skill, false, &env);
    // The agent dir holds a legacy link plus a real skill of its own.
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join(".agents/skills/pdf"),
        tmp.path().join(".claude/skills/pdf"),
    )
    .unwrap();
    let agent = get_agent("claude-code").unwrap();

    match link_agent(agent, false, &env, true) {
        LinkOutcome::Migrated {
            moved,
            skipped,
            backup_dir,
            ..
        } => {
            assert_eq!(moved, vec!["my-skill"]);
            assert!(skipped.is_empty());
            assert!(backup_dir.is_some(), "legacy link stays parked");
        }
        other => panic!("expected Migrated, got {other:?}"),
    }
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    let parked_link = tmp
        .path()
        .join(".agents/backup-skills/claude-code/skills/pdf");
    assert!(parked_link.is_symlink());
}

#[test]
fn unlink_agent_restores_parked_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    fs::write(tmp.path().join(".claude/skills/notes.txt"), "n").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Unlinked {
            restored,
            restored_from,
        } => {
            assert_eq!(sorted(restored), vec!["my-skill", "notes.txt"]);
            assert_eq!(
                restored_from,
                Some(tmp.path().join(".agents/backup-skills/claude-code/skills"))
            );
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    // Content is back in a real dir; the slot is gone.
    let dir = tmp.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert!(dir.join("my-skill/SKILL.md").exists());
    assert!(dir.join("notes.txt").exists());
    assert!(
        !tmp.path()
            .join(".agents/backup-skills/claude-code")
            .exists()
    );
}

#[test]
fn unlink_agent_removes_link_and_recreates_empty_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
    let agent = get_agent("windsurf").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Unlinked {
            restored,
            restored_from,
        } => {
            assert!(restored.is_empty());
            assert!(restored_from.is_none());
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    let dir = tmp.path().join(".windsurf/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert!(fs::read_dir(&dir).unwrap().count() == 0);
}

#[test]
fn unlink_agent_recovers_missing_dir_from_backup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    // Simulate the link being removed without an unlink: the dir is gone
    // but the backup is pending.
    fs::remove_file(tmp.path().join(".claude/skills")).unwrap();

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Unlinked { restored, .. } => {
            assert_eq!(restored, vec!["my-skill"]);
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    assert!(tmp.path().join(".claude/skills/my-skill/SKILL.md").exists());
    assert!(
        !tmp.path()
            .join(".agents/backup-skills/claude-code")
            .exists()
    );
}

#[test]
fn unlink_agent_replaces_empty_real_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    // The user removed the link and left an empty real dir behind.
    fs::remove_file(tmp.path().join(".claude/skills")).unwrap();
    fs::create_dir(tmp.path().join(".claude/skills")).unwrap();

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Unlinked { restored, .. } => {
            assert_eq!(restored, vec!["my-skill"]);
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    assert!(tmp.path().join(".claude/skills/my-skill/SKILL.md").exists());
}

#[test]
fn unlink_agent_blocked_by_real_dir_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    // The user removed the link and put their own content in the way.
    fs::remove_file(tmp.path().join(".claude/skills")).unwrap();
    let dir = tmp.path().join(".claude/skills");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("mine.txt"), "x").unwrap();

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Failed { error } => {
            assert!(error.contains("restore blocked"), "error: {error}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // Nothing was touched: user content and backup both intact.
    assert!(dir.join("mine.txt").exists());
    assert!(
        tmp.path()
            .join(".agents/backup-skills/claude-code/skills/my-skill")
            .exists()
    );
}

#[test]
fn unlink_agent_leaves_real_dirs_and_foreign_links_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        unlink_agent(agent, false, &env),
        LinkOutcome::NotLinked
    ));
    assert!(tmp.path().join(".claude/skills/my-skill").exists());

    // Universal agents never link.
    let uni = get_agent("amp").unwrap();
    assert!(matches!(
        unlink_agent(uni, false, &env),
        LinkOutcome::NotLinked
    ));
}

#[test]
fn unlink_agent_keeps_migrated_skills_in_canonical() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let existing = tmp.path().join(".claude/skills/my-skill");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("SKILL.md"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, true),
        LinkOutcome::Migrated { .. }
    ));

    match unlink_agent(agent, false, &env) {
        LinkOutcome::Unlinked { restored, .. } => {
            assert!(restored.is_empty(), "migrated skills are not restored");
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    // The migrated skill stays in the canonical dir (managed by `remove`).
    assert!(tmp.path().join(".agents/skills/my-skill/SKILL.md").exists());
    let dir = tmp.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
}

#[test]
fn is_agent_linked_reflects_dir_links() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    let agent = get_agent("windsurf").unwrap();
    assert!(!is_agent_linked(agent, false, &env));
    fs::create_dir_all(tmp.path().join(".windsurf")).unwrap();
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert!(is_agent_linked(agent, false, &env));
    // Universal agents are always "linked" (canonical is their dir).
    assert!(is_agent_linked(get_agent("amp").unwrap(), false, &env));
}

#[test]
fn private_content_classifies_skills_and_others() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    fs::write(tmp.path().join(".claude/skills/my-skill/SKILL.md"), "x").unwrap();
    fs::write(tmp.path().join(".claude/skills/README.txt"), "x").unwrap();
    let agent = get_agent("claude-code").unwrap();

    let (skills, others) = private_content(agent, false, &env);
    assert_eq!(skills, vec!["my-skill"]);
    assert_eq!(others, vec!["README.txt"]);

    // A linked (or foreign-symlink) skills dir is not private content.
    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    assert_eq!(
        private_content(agent, false, &env),
        (Vec::new(), Vec::new())
    );
}

#[test]
fn pending_backup_reports_parked_slot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env = env_at(&tmp);
    fs::create_dir_all(tmp.path().join(".claude/skills/my-skill")).unwrap();
    let agent = get_agent("claude-code").unwrap();
    assert!(pending_backup(agent, false, &env).is_none());

    assert!(matches!(
        link_agent(agent, false, &env, false),
        LinkOutcome::Linked { .. }
    ));
    let (parked, items) = pending_backup(agent, false, &env).expect("pending backup");
    assert_eq!(
        parked,
        tmp.path().join(".agents/backup-skills/claude-code/skills")
    );
    assert_eq!(items, vec!["my-skill"]);

    // After unlink the slot is gone.
    assert!(matches!(
        unlink_agent(agent, false, &env),
        LinkOutcome::Unlinked { .. }
    ));
    assert!(pending_backup(agent, false, &env).is_none());
}
