//! End-to-end tests for the `disable` and `enable` commands: move/restore, --all,
//! idempotency, and list status.

mod common;

use predicates::prelude::*;

use common::TestProject;

fn add_skill(p: &TestProject, rel_dir: &str, name: &str) {
    let src = p.write_skill_source(rel_dir, name);
    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn disable_moves_skill_out_of_canonical_dir() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");

    p.skills()
        .args(["disable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Disabled pdf"));

    p.assert_absent(".agents/skills/pdf");
    p.assert_exists(".agents/disabled-skills/pdf/SKILL.md");
}

#[test]
fn enable_moves_skill_back_into_canonical_dir() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");
    p.skills().args(["disable", "pdf"]).assert().success();

    p.skills()
        .args(["enable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Enabled pdf"));

    p.assert_exists(".agents/skills/pdf/SKILL.md");
    p.assert_absent(".agents/disabled-skills/pdf");
}

#[test]
fn disable_is_idempotent() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");
    p.skills().args(["disable", "pdf"]).assert().success();

    p.skills()
        .args(["disable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already disabled"));
}

#[test]
fn enable_is_idempotent() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");

    p.skills()
        .args(["enable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already enabled"));
}

#[test]
fn disable_missing_skill_reports_not_found() {
    let p = TestProject::new();
    p.skills()
        .args(["disable", "nope"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not found"));
}

#[test]
fn disable_all_disables_every_enabled_skill() {
    let p = TestProject::new();
    add_skill(&p, "s1", "alpha");
    add_skill(&p, "s2", "beta");

    p.skills().args(["disable", "--all"]).assert().success();

    p.assert_absent(".agents/skills/alpha");
    p.assert_absent(".agents/skills/beta");
    p.assert_exists(".agents/disabled-skills/alpha/SKILL.md");
    p.assert_exists(".agents/disabled-skills/beta/SKILL.md");
}

#[test]
fn enable_all_restores_every_disabled_skill() {
    let p = TestProject::new();
    add_skill(&p, "s1", "alpha");
    add_skill(&p, "s2", "beta");
    p.skills().args(["disable", "--all"]).assert().success();

    p.skills().args(["enable", "--all"]).assert().success();

    p.assert_exists(".agents/skills/alpha/SKILL.md");
    p.assert_exists(".agents/skills/beta/SKILL.md");
    p.assert_absent(".agents/disabled-skills/alpha");
    p.assert_absent(".agents/disabled-skills/beta");
}

#[test]
fn list_shows_disabled_status_after_disable() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");
    p.skills().args(["disable", "pdf"]).assert().success();

    p.skills()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("disabled"));
}

#[test]
fn list_json_reports_enabled_field() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");

    p.skills()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"enabled\": true"));

    p.skills().args(["disable", "pdf"]).assert().success();

    p.skills()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"enabled\": false"));
}

/// Simulate a third-party agent re-installing a skill whose copy is still parked in
/// the disabled dir: the name then exists in both dirs.
fn third_party_reinstall(p: &TestProject, name: &str) {
    let dir = p.path().join(".agents/skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: reinstalled\n---\n\n# {name}\n"),
    )
    .unwrap();
}

#[test]
fn enable_overwrites_the_enabled_copy_of_a_disabled_skill() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");
    p.skills().args(["disable", "pdf"]).assert().success();
    third_party_reinstall(&p, "pdf");

    p.skills()
        .args(["enable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Enabled pdf"));

    // Exactly one copy left, and it is the one being enabled.
    p.assert_exists(".agents/skills/pdf/SKILL.md");
    p.assert_absent(".agents/disabled-skills/pdf");
    assert!(
        !p.read(".agents/skills/pdf/SKILL.md")
            .contains("reinstalled")
    );
}

#[test]
fn disable_overwrites_the_parked_copy_of_a_reinstalled_skill() {
    let p = TestProject::new();
    add_skill(&p, "my-skill", "pdf");
    p.skills().args(["disable", "pdf"]).assert().success();
    third_party_reinstall(&p, "pdf");

    p.skills()
        .args(["disable", "pdf"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Disabled pdf"));

    p.assert_absent(".agents/skills/pdf");
    p.assert_exists(".agents/disabled-skills/pdf/SKILL.md");
    assert!(
        p.read(".agents/disabled-skills/pdf/SKILL.md")
            .contains("reinstalled")
    );
}

#[test]
fn enable_replaces_a_differently_named_copy_of_the_same_skill() {
    let p = TestProject::new();
    // An adopted skill keeps its original dir name; the parked `pdf-master` is the
    // same skill, so enabling it must leave one copy in the canonical dir.
    let adopted = p.path().join(".agents/skills/PDF Master");
    std::fs::create_dir_all(&adopted).unwrap();
    std::fs::write(adopted.join("SKILL.md"), common::skill_md("PDF Master")).unwrap();
    let parked = p.path().join(".agents/disabled-skills/pdf-master");
    std::fs::create_dir_all(&parked).unwrap();
    std::fs::write(parked.join("SKILL.md"), common::skill_md("pdf-master")).unwrap();

    p.skills().args(["enable", "pdf-master"]).assert().success();

    p.assert_absent(".agents/skills/PDF Master");
    p.assert_exists(".agents/skills/pdf-master/SKILL.md");
    p.assert_absent(".agents/disabled-skills/pdf-master");
}
