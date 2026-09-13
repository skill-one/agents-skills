//! End-to-end tests for the `remove` command: single removal, --all, no-args, nonexistent skill.

mod common;

use predicates::prelude::*;

use common::TestProject;

#[test]
fn remove_deletes_installed_skill() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();
    p.assert_exists(".agents/skills/pdf");

    p.skills()
        .args(["remove", "pdf", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Successfully removed 1 skill"));

    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn remove_all_flag_removes_everything() {
    let p = TestProject::new();
    let a = p.write_skill_source("skill-a", "pdf");
    let b = p.write_skill_source("skill-b", "docx");

    p.skills()
        .args(["add", a.to_str().unwrap(), "--project", "."])
        .assert()
        .success();
    p.skills()
        .args(["add", b.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["remove", "--all", "--project", "."])
        .assert()
        .success();

    p.assert_absent(".agents/skills/pdf");
    p.assert_absent(".agents/skills/docx");
}

#[test]
fn remove_without_args_prints_installed_list() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["remove", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed skills"))
        .stdout(predicate::str::contains("pdf"));

    p.assert_exists(".agents/skills/pdf");
}

#[test]
fn remove_nonexistent_prints_no_match() {
    let p = TestProject::new();
    p.skills()
        .args(["remove", "ghost", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("No matching skills found"));
}

#[test]
fn remove_deletes_disabled_skill_without_lock_entry() {
    let p = TestProject::new();
    // Simulate a skill parked in the disabled dir by a third-party tool: no lockfile entry.
    let disabled = p.path().join(".agents/disabled-skills/legacy");
    std::fs::create_dir_all(&disabled).unwrap();
    std::fs::write(
        disabled.join("SKILL.md"),
        "---\nname: legacy\ndescription: does legacy\n---\n\n# legacy\n",
    )
    .unwrap();

    p.skills()
        .args(["remove", "legacy", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Successfully removed 1 skill"));

    p.assert_absent(".agents/disabled-skills/legacy");
}

#[test]
fn remove_all_deletes_disabled_skills() {
    let p = TestProject::new();
    // An enabled skill plus a disabled one (no lock entry for the disabled).
    let src = p.write_skill_source("my-skill", "pdf");
    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();
    let disabled = p.path().join(".agents/disabled-skills/legacy");
    std::fs::create_dir_all(&disabled).unwrap();
    std::fs::write(
        disabled.join("SKILL.md"),
        "---\nname: legacy\ndescription: does legacy\n---\n\n# legacy\n",
    )
    .unwrap();

    p.skills()
        .args(["remove", "--all", "--project", "."])
        .assert()
        .success();

    p.assert_absent(".agents/skills/pdf");
    p.assert_absent(".agents/disabled-skills/legacy");
}
