//! Shared fixtures and helpers for integration tests.
//!
//! Each `cli_*.rs` integration test imports this via `mod common;`, avoiding duplication of
//! helpers like `write_skill_source` across files (DRY). Following Rust ecosystem convention
//! (e.g. `assert_cmd`'s examples, `ripgrep`'s `tests/util`), common fixtures live in `common/mod.rs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

/// A reusable workspace: a temp dir + an `agents-skills` command builder pointing at it.
///
/// `TempDir` cleans up on drop, so tests never need to delete temp files manually.
pub struct TestProject {
    pub dir: TempDir,
}

impl TestProject {
    pub fn new() -> Self {
        TestProject {
            dir: TempDir::new().expect("create temp dir"),
        }
    }

    /// Current project root path.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Return an `agents-skills` command pointed at the scratch environment:
    /// `HOME` is the temp dir (so the canonical dir is `<tmp>/.agents/skills`)
    /// and `current_dir` is the same temp dir.
    pub fn skills(&self) -> Command {
        let mut cmd = Command::cargo_bin("agents-skills").expect("resolve agents-skills binary");
        cmd.current_dir(self.path());
        cmd.env("HOME", self.path());
        cmd
    }

    /// Create a source skill dir with standard frontmatter, returning its absolute path.
    ///
    /// The generated content looks like:
    /// ```markdown
    /// ---
    /// name: <name>
    /// description: does <name>
    /// ---
    ///
    /// # <name>
    /// ```
    pub fn write_skill_source(&self, rel_dir: &str, name: &str) -> PathBuf {
        write_skill_source(self.path(), rel_dir, name)
    }

    /// Read the contents of a project-relative path as a string.
    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).expect("read file")
    }

    /// Assert a project-relative path exists.
    pub fn assert_exists(&self, rel: &str) {
        assert!(
            self.path().join(rel).exists(),
            "expected {rel} to exist under {}",
            self.path().display()
        );
    }

    /// Assert a project-relative path does not exist.
    pub fn assert_absent(&self, rel: &str) {
        assert!(
            !self.path().join(rel).exists(),
            "expected {rel} to NOT exist under {}",
            self.path().display()
        );
    }
}

/// Create a source skill dir with standard frontmatter under `root`.
pub fn write_skill_source(root: &Path, rel_dir: &str, name: &str) -> PathBuf {
    let dir = root.join(rel_dir);
    std::fs::create_dir_all(&dir).expect("create skill dir");
    std::fs::write(dir.join("SKILL.md"), skill_md(name)).expect("write SKILL.md");
    dir
}

/// Generate standard SKILL.md content.
pub fn skill_md(name: &str) -> String {
    format!("---\nname: {name}\ndescription: does {name}\n---\n\n# {name}\n")
}
