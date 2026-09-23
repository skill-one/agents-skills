//! Shared fixtures for unit tests (compiled only under `cfg(test)`).
//!
//! Centralizes helpers reused by multiple module tests — `Env` construction, `SKILL.md`
//! directory generation — eliminating duplication across `agents` / `install` / `discover` (DRY).

use std::path::{Path, PathBuf};

use crate::core::agents::Env;
use crate::core::discover::{Skill, read_skill};

/// Construct an `Env` in a temp dir: home=cwd=tmp, config=tmp/config.
///
/// Hermetic by construction: system-location probes are off and injected env
/// vars are empty, so detection never consults the real machine or process env.
pub fn env_at(tmp: &tempfile::TempDir) -> Env {
    let mut env = Env::new(tmp.path(), tmp.path().join("config"), tmp.path());
    env.set_probe_system_dirs(false);
    env.set_vars(std::collections::HashMap::new());
    env
}

/// Generate a standard `SKILL.md` dir under `root/rel_dir`, returning the SKILL.md path.
pub fn write_skill_md(root: &Path, rel_dir: &str, name: &str) -> PathBuf {
    let dir = root.join(rel_dir);
    std::fs::create_dir_all(&dir).expect("create skill dir");
    let md = dir.join("SKILL.md");
    std::fs::write(&md, skill_frontmatter(name)).expect("write SKILL.md");
    md
}

/// Generate standard frontmatter content (short body, for discover tests).
pub fn skill_frontmatter(name: &str) -> String {
    format!("---\nname: {name}\ndescription: does {name}\n---\n\n# {name}\n\nBody text.\n")
}

/// Generate a SKILL.md under `dir` and read it back as a `Skill` (for install
/// tests). `dir`'s file name is the skill name; `name` only sets frontmatter.
pub fn write_and_parse_skill(dir: &Path, name: &str) -> Skill {
    std::fs::create_dir_all(dir).expect("create skill dir");
    let md = dir.join("SKILL.md");
    std::fs::write(&md, skill_frontmatter(name)).expect("write SKILL.md");
    read_skill(dir, false).expect("read skill dir")
}
