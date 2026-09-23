//! End-to-end tests for the `add` command: install and error cases.

mod common;

use predicates::prelude::*;

use common::TestProject;

#[test]
fn add_local_path_installs_to_canonical() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));

    p.assert_exists(".agents/skills/pdf/SKILL.md");
    p.assert_absent("skills-lock.json");
}

#[test]
fn add_uses_the_directory_name_not_frontmatter_name() {
    let p = TestProject::new();
    // The directory is `my-skill`; the frontmatter `name` differs and is ignored.
    let src = p.write_skill_source("my-skill", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Skill:").and(predicate::str::contains("my-skill")));

    p.assert_exists(".agents/skills/my-skill/SKILL.md");
    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn add_skips_an_already_installed_skill() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");

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
    let src = p.write_skill_source("pdf", "pdf");

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
fn add_local_dir_without_skill_md_is_rejected() {
    let p = TestProject::new();
    // A multi-skill repo root has no direct SKILL.md: point at the skill dir.
    let _src = p.write_skill_source("repo/skills/pdf", "pdf");
    let root = p.path().join("repo");

    p.skills()
        .args(["add", root.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("Not a skill directory"));

    // Pointing directly at the skill directory works.
    let skill_dir = p.path().join("repo/skills/pdf");
    p.skills()
        .args(["add", skill_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed 1 skill"));
}

#[test]
fn bare_owner_repo_is_rejected_without_network() {
    let p = TestProject::new();

    // The missing `@<skill>` is a parse error, reported before any network I/O.
    p.skills()
        .args(["add", "acme/skills"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("Missing skill selector"))
        .stdout(predicate::str::contains("acme/skills@<skill>"));
}

#[test]
fn subpath_and_url_sources_are_rejected() {
    let p = TestProject::new();

    p.skills()
        .args(["add", "acme/skills/skills/pdf"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("subpaths"));

    p.skills()
        .args(["add", "https://github.com/acme/skills"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("Full URLs"));
}

#[test]
fn removed_flags_are_rejected() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");

    // Project scope was removed.
    p.skills()
        .args(["add", src.to_str().unwrap(), "--project", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    // Repository-wide listing was removed.
    p.skills()
        .args(["add", src.to_str().unwrap(), "--list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    // The --skill selector was folded into `owner/repo@<skill>`.
    p.skills()
        .args(["add", src.to_str().unwrap(), "--skill", "pdf"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    p.assert_absent(".agents/skills/pdf");
}

#[test]
fn full_depth_flag_is_rejected() {
    let p = TestProject::new();

    // Deliberately removed: there is no tree discovery to override anymore.
    p.skills()
        .args(["add", p.path().to_str().unwrap(), "--full-depth"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn only_one_source_argument_is_accepted() {
    let p = TestProject::new();
    let a = p.write_skill_source("alpha", "alpha");
    let b = p.write_skill_source("beta", "beta");

    // One add installs exactly one skill; a second positional is a CLI error.
    p.skills()
        .args(["add", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn ref_flag_is_rejected_for_local_sources() {
    let p = TestProject::new();
    let src = p.write_skill_source("pdf", "pdf");

    p.skills()
        .args(["add", src.to_str().unwrap(), "--ref", "v1.2"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains(
            "--ref can only pin a GitHub source",
        ));

    p.assert_absent(".agents/skills/pdf");
}
