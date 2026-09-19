//! End-to-end tests for the `add` command: install, --list, error cases.

mod common;

use predicates::prelude::*;

use common::TestProject;

#[test]
fn add_local_path_installs_to_canonical() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));

    p.assert_exists(".agents/skills/pdf/SKILL.md");
    p.assert_absent("skills-lock.json");
}

#[test]
fn add_skips_an_already_installed_skill() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    // Installing the same name again is reported, not silently repeated.
    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped (already installed)"));
}

#[test]
fn subcommand_aliases_are_rejected() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    // Deliberately alias-free: full command names only (minimal interface).
    for alias in [
        "a", "i", "install", "rm", "r", "ls", "d", "e", "upgrade", "check",
    ] {
        p.skills()
            .args([alias, src.to_str().unwrap()])
            .assert()
            .failure()
            .stdout(predicate::str::contains("Unknown command"));
    }

    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn add_list_flag_prints_without_installing() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .arg("add")
        .arg(&src)
        .arg("--list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Available Skills"))
        .stdout(predicate::str::contains("pdf"));

    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn add_missing_local_path_exits_nonzero() {
    let p = TestProject::new();
    let missing = p.path().join("does-not-exist");

    p.skills()
        .args(["add", missing.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("Local path does not exist"));
}

#[test]
fn project_flag_is_rejected() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    // Project scope was removed: the flag no longer exists.
    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn full_depth_flag_is_rejected() {
    let p = TestProject::new();

    // Deliberately removed: discovery trusts its conventions, no full-tree override.
    p.skills()
        .args(["add", p.path().to_str().unwrap(), "--full-depth"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn add_installs_every_source_argument() {
    let p = TestProject::new();
    let a = p.write_skill_source("src-a", "alpha");
    let b = p.write_skill_source("src-b", "beta");

    // Multiple <source...> args: every source is installed, none silently dropped.
    p.skills()
        .args(["add", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));

    p.assert_exists(".agents/skills/alpha/SKILL.md");
    p.assert_exists(".agents/skills/beta/SKILL.md");
}
