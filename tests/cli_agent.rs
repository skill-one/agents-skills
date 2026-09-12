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
    // claude-code links even without .claude/ (historical exception).
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
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
fn agent_link_parks_content_and_migrate_adopts_it() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    // Plain link parks the existing skill in the backup slot and links anyway.
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"))
        .stdout(predicate::str::contains("parked"));

    let slot = p.path().join(".agents/backup-skills/claude-code");
    assert!(slot.join("skills/my-skill/SKILL.md").exists());
    assert!(p.path().join(".claude/skills").is_symlink());
    assert!(!p.path().join(".agents/skills").exists());

    // --migrate pulls the parked skill into the canonical dir.
    p.skills()
        .args([
            "agent",
            "--link",
            "claude-code",
            "--project",
            ".",
            "--migrate",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("migrated"));

    p.assert_exists(".agents/skills/my-skill/SKILL.md");
    assert!(p.path().join(".claude/skills").is_symlink());
    // The slot keeps only non-skill leftovers (none here), so it is gone.
    assert!(!slot.exists());
}

#[test]
fn agent_link_parks_stray_files_and_unlink_restores_them() {
    let p = TestProject::new();
    // A real file is not a skill: it is parked (not migrated), and linking succeeds.
    std::fs::create_dir_all(p.path().join(".claude/skills")).unwrap();
    std::fs::write(p.path().join(".claude/skills/README.txt"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("parked existing content"));

    assert!(p.path().join(".claude/skills").is_symlink());
    assert!(
        p.path()
            .join(".agents/backup-skills/claude-code/skills/README.txt")
            .exists()
    );

    // Unlink restores the parked file into a real dir.
    p.skills()
        .args(["agent", "--unlink", "claude-code", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("restored README.txt"));

    assert!(p.path().join(".claude/skills/README.txt").exists());
    assert!(!p.path().join(".claude/skills").is_symlink());
    assert!(!p.path().join(".agents/backup-skills/claude-code").exists());
}

#[test]
fn agent_unlink_restores_real_dir() {
    let p = TestProject::new();
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["agent", "--unlink", "claude-code", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("unlinked"));

    let dir = p.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
}

#[test]
fn agent_link_unlink_roundtrip_restores_parked_skills() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();
    assert!(p.path().join(".claude/skills").is_symlink());

    p.skills()
        .args(["agent", "--unlink", "claude-code", "--project", "."])
        .assert()
        .success();

    // The parked skill is back in a real dir; the backup slot is gone.
    let dir = p.path().join(".claude/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert!(dir.join("my-skill/SKILL.md").exists());
    assert!(!p.path().join(".agents/backup-skills/claude-code").exists());
}

#[test]
fn agent_status_prints_installed_agents_and_link_state() {
    let p = TestProject::new();
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();

    // CodeBuddy is detected via `.codebuddy` in the cwd: installed but not linked.
    std::fs::create_dir_all(p.path().join(".codebuddy")).unwrap();

    p.skills()
        .args(["agent", "--status", "--project", "."])
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
    // CodeBuddy is detected via `.codebuddy` in the cwd: installed but not linked.
    std::fs::create_dir_all(p.path().join(".codebuddy/skills/pdf")).unwrap();
    std::fs::write(
        p.path().join(".codebuddy/skills/pdf/SKILL.md"),
        "---\nname: pdf\ndescription: does pdf\n---\nbody",
    )
    .unwrap();
    std::fs::write(p.path().join(".codebuddy/skills/README.txt"), "x").unwrap();

    p.skills()
        .args(["agent", "--status", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("CodeBuddy"))
        .stdout(predicate::str::contains("not linked"))
        .stdout(predicate::str::contains("private skills: pdf"))
        .stdout(predicate::str::contains("other files: README.txt"));
}

#[test]
fn agent_status_shows_pending_backup_slot() {
    let p = TestProject::new();
    let existing = p.path().join(".claude/skills/my-skill");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "x").unwrap();

    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();

    // The agent is linked now; remove the link manually to simulate a
    // half-disconnected state — the parked slot must stay visible in status.
    std::fs::remove_file(p.path().join(".claude/skills")).unwrap();

    p.skills()
        .env("HOME", p.path())
        .args(["agent", "--status", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Claude Code"))
        .stdout(predicate::str::contains("not linked"))
        .stdout(predicate::str::contains("backup parked at"))
        .stdout(predicate::str::contains("my-skill"));
}

#[test]
fn agent_status_orders_canonical_agents_first() {
    let p = TestProject::new();
    // codex is canonical (universal); claude-code is non-canonical but linked.
    std::fs::create_dir_all(p.path().join(".codex")).unwrap();
    std::fs::create_dir_all(p.path().join(".claude")).unwrap();
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();

    let out = p
        .skills()
        .env("HOME", p.path())
        .args(["agent", "--status", "--project", "."])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Same order as the library: canonical (universal) agents render first.
    let codex = stdout.find("Codex").expect("codex listed");
    let claude = stdout.find("Claude Code").expect("claude listed");
    assert!(codex < claude);
}

#[test]
fn agent_status_marks_universal_agents_as_canonical() {
    let p = TestProject::new();
    // Warp is a universal agent (reads `.agents/skills` natively); pretend it's installed.
    std::fs::create_dir_all(p.path().join(".warp")).unwrap();

    p.skills()
        .env("HOME", p.path())
        .args(["agent", "--status", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Warp"))
        .stdout(predicate::str::contains(") — canonical"));
}

#[test]
fn agent_global_link_connects_project_universal_agent_vendor_dir() {
    let p = TestProject::new();
    // Antigravity is universal at project scope but reads the vendor-specific
    // ~/.gemini/config/skills dir at GLOBAL scope (HOME is faked for hermeticity).
    std::fs::create_dir_all(p.path().join(".gemini/config")).unwrap();

    p.skills()
        .env("HOME", p.path())
        .args(["agent", "--link", "antigravity"])
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"));

    let link = p.path().join(".gemini/config/skills");
    assert!(link.is_symlink());

    // Global status reports it as linked (not canonical).
    p.skills()
        .env("HOME", p.path())
        .args(["agent", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("antigravity) — linked"));

    // Unlink restores a real empty dir.
    p.skills()
        .env("HOME", p.path())
        .args(["agent", "--unlink", "antigravity"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unlinked"));
    assert!(link.is_dir());
    assert!(!link.is_symlink());
}

#[test]
fn agent_status_conflicts_with_unlink_and_migrate() {
    let p = TestProject::new();

    p.skills()
        .args(["agent", "--status", "--unlink"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));

    p.skills()
        .args(["agent", "--status", "--migrate"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));

    p.skills()
        .args(["agent", "--unlink", "--migrate"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn add_then_agent_link_ensures_agent_links() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));

    p.assert_exists(".agents/skills/pdf/SKILL.md");
    // `add` only installs into the canonical dir — no agent link yet.
    assert!(!p.path().join(".claude/skills").is_symlink());

    // `agent --link` exposes the canonical dir to the agent.
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
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
    let src = p.write_skill_source("my-skill", "pdf");
    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();
    p.skills()
        .args(["agent", "--link", "claude-code", "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["remove", "pdf", "--project", "."])
        .assert()
        .success();

    p.assert_absent(".agents/skills/pdf");
    // The dir link remains, but the skill is gone (no dead per-skill links).
    assert!(p.path().join(".claude/skills").is_symlink());
    assert!(!p.path().join(".claude/skills/pdf").exists());
}
