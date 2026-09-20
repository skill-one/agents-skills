//! End-to-end tests for the `list` command: empty state, plain output, JSON.

mod common;

use predicates::prelude::*;

use common::TestProject;

#[test]
fn list_empty_prints_hint() {
    let p = TestProject::new();

    p.skills()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No skills installed."))
        .stdout(predicate::str::contains("Try: agents-skills add"));
}

#[test]
fn list_json_reports_skill_fields() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    p.skills()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"pdf\""))
        .stdout(predicate::str::contains("\"description\": \"does pdf\""))
        .stdout(predicate::str::contains("\"enabled\": true"))
        // The key is always emitted; the value may be null where the
        // filesystem records no creation time.
        .stdout(predicate::str::contains("\"installedAt\""));
}

#[test]
fn list_plain_prints_description() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    p.skills()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("does pdf"));
}

#[test]
fn list_plain_prints_skill() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    p.skills()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Skills"))
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("enabled"));
}

#[test]
fn list_plain_hides_agents() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    // Plain output shows name/path/status but no per-skill agent column;
    // agent link status is `agent --status`'s job.
    p.skills()
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("enabled"))
        .stdout(predicate::str::contains("Agents").not());
}

#[test]
fn list_reports_disabled_skills() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();
    p.skills().args(["disable", "pdf"]).assert().success();

    p.skills()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"enabled\": false"));
}
