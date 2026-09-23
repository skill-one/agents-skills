//! End-to-end tests for the `agent` command (--link / --status / --unlink) and the directory-link model.

mod common;

use predicates::prelude::*;
use std::path::Path;

use common::TestProject;

#[test]
fn agent_requires_mode_flag() {
    let p = TestProject::new();
    p.skills()
        .args(["agent", "claude-code"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn agent_link_creates_relative_dir_symlink() {
    let p = TestProject::new();
    // claude-code links even without ~/.claude (historical exception).
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"));

    let link = p.path().join(".claude/skills");
    assert!(link.is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("../.agents/skills")
    );
}

#[test]
fn agent_link_adopts_existing_skills() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    // Linking adopts the existing skill into the canonical dir.
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"))
        .stdout(predicate::str::contains(
            "adopted into the canonical dir: my-skill",
        ));

    p.assert_exists(".agents/skills/my-skill/SKILL.md");
    assert!(p.path().join(".claude/skills").is_symlink());
}

#[test]
fn agent_link_quarantines_non_skill_files() {
    let p = TestProject::new();
    // A real file is not a skill: it is quarantined under .misc/, linking succeeds.
    std::fs::create_dir_all(p.path().join(".claude/skills")).unwrap();
    std::fs::write(p.path().join(".claude/skills/README.txt"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"))
        .stdout(predicate::str::contains(
            "moved non-skill files into .misc/: README.txt",
        ));

    assert!(p.path().join(".claude/skills").is_symlink());
    assert!(
        p.path()
            .join(".agents/skills/.misc/claude-code/README.txt")
            .exists()
    );
}

#[test]
fn agent_unlink_restores_real_dir() {
    let p = TestProject::new();
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();

    p.skills()
        .args(["agent", "--unlink", "claude-code"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unlinked"));

    let dir = p.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
}

#[test]
fn agent_link_unlink_roundtrip_keeps_adopted_skills() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();
    assert!(p.path().join(".claude/skills").is_symlink());

    p.skills()
        .args(["agent", "--unlink", "claude-code"])
        .assert()
        .success();

    // The adopted skill stays in the canonical dir; the agent dir is empty again.
    p.assert_exists(".agents/skills/my-skill/SKILL.md");
    let dir = p.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn agent_status_prints_installed_agents_and_link_state() {
    let p = TestProject::new();
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();

    // CodeBuddy is detected via ~/.codebuddy: installed but not linked.
    std::fs::create_dir_all(p.path().join(".codebuddy")).unwrap();

    p.skills()
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Agent link status"))
        .stdout(predicate::str::contains("Claude Code"))
        .stdout(predicate::str::contains(") — linked"))
        .stdout(predicate::str::contains("CodeBuddy"))
        .stdout(predicate::str::contains("codebuddy) — not linked"));
}

#[test]
fn agent_status_classifies_unlinked_agents_private_content() {
    let p = TestProject::new();
    // CodeBuddy is detected via ~/.codebuddy: installed but not linked.
    std::fs::create_dir_all(p.path().join(".codebuddy/skills/pdf")).unwrap();
    std::fs::write(
        p.path().join(".codebuddy/skills/pdf/SKILL.md"),
        "---\nname: pdf\ndescription: does pdf\n---\nbody",
    )
    .unwrap();
    std::fs::write(p.path().join(".codebuddy/skills/README.txt"), "x").unwrap();

    p.skills()
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CodeBuddy"))
        .stdout(predicate::str::contains("not linked"))
        .stdout(predicate::str::contains("private skills: pdf"))
        .stdout(predicate::str::contains("other files: README.txt"));
}

#[test]
fn agent_status_reports_manually_unlinked_agent() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();

    // Remove the link manually: status must fall back to "not linked".
    std::fs::remove_file(p.path().join(".claude/skills")).unwrap();

    p.skills()
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Claude Code"))
        .stdout(predicate::str::contains("not linked"));
}

#[test]
fn agent_status_orders_canonical_agents_first() {
    let p = TestProject::new();
    // cline is canonical (its skills dir is ~/.agents/skills); claude-code is
    // non-canonical but linked, and precedes cline in the static agent table —
    // so this proves the canonical-first ordering rather than a coincidence.
    std::fs::create_dir_all(p.path().join(".cline")).unwrap();
    std::fs::create_dir_all(p.path().join(".claude")).unwrap();
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();

    let out = p.skills().args(["agent", "--status"]).output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Same order as the library: canonical agents render first.
    let cline = stdout.find("Cline").expect("cline listed");
    let claude = stdout.find("Claude Code").expect("claude listed");
    assert!(cline < claude);
}

#[test]
fn agent_status_marks_canonical_agents() {
    let p = TestProject::new();
    // Warp's skills dir is ~/.agents/skills itself; pretend it's installed.
    std::fs::create_dir_all(p.path().join(".warp")).unwrap();

    p.skills()
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Warp"))
        .stdout(predicate::str::contains(") — canonical"));
}

#[test]
fn agent_link_connects_vendor_specific_dir() {
    let p = TestProject::new();
    // Antigravity reads the vendor-specific ~/.gemini/config/skills dir.
    std::fs::create_dir_all(p.path().join(".gemini/config")).unwrap();

    p.skills()
        .args(["agent", "--link", "antigravity"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"));

    let link = p.path().join(".gemini/config/skills");
    assert!(link.is_symlink());

    // Status reports it as linked (not canonical).
    p.skills()
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("antigravity) — linked"));

    // Unlink restores a real empty dir.
    p.skills()
        .args(["agent", "--unlink", "antigravity"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unlinked"));
    assert!(link.is_dir());
    assert!(!link.is_symlink());
}

#[test]
fn agent_status_conflicts_with_unlink() {
    let p = TestProject::new();

    p.skills()
        .args(["agent", "--status", "--unlink"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn add_then_agent_link_ensures_agent_links() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));

    p.assert_exists(".agents/skills/pdf/SKILL.md");
    // `add` only installs into the canonical dir — no agent link yet.
    assert!(!p.path().join(".claude/skills").is_symlink());

    // `agent --link` exposes the canonical dir to the agent.
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();
    let link = p.path().join(".claude/skills");
    assert!(link.is_symlink());
    // The skill is visible through the agent link.
    assert!(link.join("pdf/SKILL.md").exists());
}

#[test]
fn remove_skill_disappears_from_linked_agents() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");
    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();
    p.skills()
        .args(["agent", "--link", "claude-code"])
        .assert()
        .success();

    p.skills().args(["remove", "pdf"]).assert().success();

    p.assert_absent(".agents/skills/pdf");
    // The dir link remains, but the skill is gone (no dead per-skill links).
    assert!(p.path().join(".claude/skills").is_symlink());
    assert!(!p.path().join(".claude/skills/pdf").exists());
}
