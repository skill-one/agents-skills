//! Skill identity and SKILL.md frontmatter helpers.
//!
//! A skill's identity is its **directory name** — never the `name` field in
//! SKILL.md, which is ignored. The manifest is parsed best-effort for the
//! description shown by `list`; a missing or unparseable description is empty,
//! never an error. There is no repository-wide discovery: `add` points at one
//! skill directory, and the GitHub matcher locates it by directory name.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A parsed skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name — the directory's file name.
    pub name: String,
    /// Skill description from frontmatter (empty when absent/unparseable).
    pub description: String,
    /// Directory containing SKILL.md.
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: Option<noyalib::Value>,
}

/// Split the `---`-delimited frontmatter, returning the YAML data.
/// Returns None when there is no frontmatter.
fn split_frontmatter(raw: &str) -> Option<&str> {
    let rest = raw
        .strip_prefix("---\r\n")
        .or_else(|| raw.strip_prefix("---\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// Read the description from a SKILL.md path; "" when the file is missing or
/// the frontmatter is absent/unparseable.
pub fn read_description(skill_md: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(skill_md) else {
        return String::new();
    };
    let Some(data) = split_frontmatter(&content) else {
        return String::new();
    };
    let Ok(fm) = noyalib::from_str::<Frontmatter>(data) else {
        return String::new();
    };
    fm.description.unwrap_or_default()
}

/// Whether the parsed frontmatter marks the skill internal.
fn is_internal(fm: &Frontmatter) -> bool {
    fm.metadata
        .as_ref()
        .and_then(|m| m.get("internal"))
        .and_then(|m| m.as_bool())
        .unwrap_or(false)
}

/// Build a [`Skill`] from its directory.
///
/// The skill name is the directory's file name; the description is read from
/// `<dir>/SKILL.md` best-effort. `None` is returned only when the directory has
/// no valid UTF-8 file name or the skill is internal and not allowed
/// (explicit selection passes `allow_internal = true`; otherwise the
/// `INSTALL_INTERNAL_SKILLS=1` opt-in applies).
pub fn read_skill(dir: &Path, allow_internal: bool) -> Option<Skill> {
    let name = dir.file_name()?.to_str()?.to_string();

    let mut description = String::new();
    if let Ok(content) = std::fs::read_to_string(dir.join("SKILL.md"))
        && let Some(data) = split_frontmatter(&content)
        && let Ok(fm) = noyalib::from_str::<Frontmatter>(data)
    {
        if is_internal(&fm) && !allow_internal && !install_internal_skills() {
            return None;
        }
        if let Some(d) = fm.description {
            description = d;
        }
    }

    Some(Skill {
        name,
        description,
        dir: dir.to_path_buf(),
    })
}

fn install_internal_skills() -> bool {
    match std::env::var("INSTALL_INTERNAL_SKILLS") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_utils::write_skill_md;

    #[test]
    fn name_is_the_directory_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "pdf", "pdf");
        let dir = tmp.path().join("pdf");
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.name, "pdf");
        assert_eq!(skill.description, "does pdf");
        assert_eq!(skill.dir, dir);
    }

    #[test]
    fn frontmatter_name_is_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_skill_md(tmp.path(), "skills/foo", "bar");
        let skill = read_skill(&tmp.path().join("skills/foo"), false).unwrap();
        assert_eq!(skill.name, "foo");
    }

    #[test]
    fn missing_description_is_empty_not_an_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("no-desc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\nbody").unwrap();
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.description, "");
    }

    #[test]
    fn invalid_yaml_and_missing_frontmatter_are_lenient() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("bad-yaml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: [unclosed\n---\nbody").unwrap();
        assert_eq!(read_skill(&dir, false).unwrap().description, "");

        let dir = tmp.path().join("no-fm");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "# just a heading\n").unwrap();
        assert_eq!(read_skill(&dir, false).unwrap().description, "");

        // Missing manifest: still a skill with an empty description.
        let dir = tmp.path().join("no-md");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_skill(&dir, false).unwrap().description, "");
    }

    #[test]
    fn quoted_and_block_description() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("pdf2");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: \"ignored\"\ndescription: |\n  Multi line\n  description here\n---\nbody",
        )
        .unwrap();
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.name, "pdf2");
        assert!(skill.description.contains("Multi line"));
    }

    #[test]
    fn internal_skill_hidden_by_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("secret");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: secret\ndescription: internal\nmetadata:\n  internal: true\n---\nbody",
        )
        .unwrap();
        assert!(read_skill(&dir, false).is_none());
        // Visible when explicitly selected.
        assert!(read_skill(&dir, true).is_some());
    }
}
