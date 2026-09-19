//! End-to-end tests for the `list` command: global default, `--project`, JSON.

mod common;

use predicates::prelude::*;

use common::TestProject;

#[test]
fn list_empty_global_prints_hint() {
    let p = TestProject::new();
    let home = p.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    // Global scope is the default; HOME is isolated so the real home is never read.
    p.skills()
        .env("HOME", &home)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No global skills found."))
        .stdout(predicate::str::contains(
            "Try listing project skills with --project",
        ));
}

#[test]
fn list_empty_project_prints_hint() {
    let p = TestProject::new();
    let home = p.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    p.skills()
        .env("HOME", &home)
        .args(["list", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("No project skills found."))
        .stdout(predicate::str::contains(
            "Try listing global skills without --project",
        ));
}

#[test]
fn list_json_reports_skill_fields() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["list", "--project", ".", "--json"])
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
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["list", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("does pdf"));
}

#[test]
fn list_plain_prints_skill() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["list", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Project Skills"))
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("enabled"));
}

#[test]
fn list_global_scope_by_default() {
    let p = TestProject::new();
    let home = p.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .env("HOME", &home)
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success();

    p.skills()
        .env("HOME", &home)
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Global Skills"))
        .stdout(predicate::str::contains("pdf"));
}

#[test]
fn list_plain_hides_agents() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    // Plain output shows name/path/status but no per-skill agent column;
    // agent link status is `agent --status`'s job.
    p.skills()
        .args(["list", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("enabled"))
        .stdout(predicate::str::contains("Agents").not());
}

#[test]
fn list_plain_prints_enabled_status() {
    let p = TestProject::new();
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .success();

    p.skills()
        .args(["list", "--project", "."])
        .assert()
        .success()
        .stdout(predicate::str::contains("enabled"));
}
