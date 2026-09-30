//! Skill identity and SKILL.md frontmatter helpers.
//!
//! A **skill** is a directory that directly contains a `SKILL.md` whose
//! frontmatter declares a non-empty `name` — the `description` is optional
//! (empty when absent). Anything else — a directory without a manifest, an
//! unparseable one, or a manifest missing the `name` — is not a skill and is
//! never discovered: not by `list`, not by `remove`/`enable`/`disable`, not
//! by the GitHub matcher, and not by a local `add`. The frontmatter `name` is
//! the skill's identity everywhere.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A parsed skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill name — the `name` declared in the SKILL.md frontmatter. The
    /// skill's identity is [`crate::core::install::slugify`] of this value.
    pub name: String,
    /// Skill description from frontmatter.
    pub description: String,
    /// Directory containing SKILL.md. Its file name is the install slot —
    /// the directory the skill is installed under, verbatim.
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    #[serde(default)]
    name: Option<String>,
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

/// The parsed frontmatter of the SKILL.md in `dir`, validated: `None` unless
/// the file exists, has parseable frontmatter, and declares a non-empty
/// `name` — the minimum that makes the directory a skill. The `description`
/// is optional and empty when absent; the flag marks `metadata.internal`.
fn validated_frontmatter(dir: &Path) -> Option<(String, String, bool)> {
    let content = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let data = split_frontmatter(&content)?;
    let fm = noyalib::from_str::<Frontmatter>(data).ok()?;
    let name = fm.name?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let description = fm.description.unwrap_or_default();
    let internal = fm
        .metadata
        .as_ref()
        .and_then(|m| m.get("internal"))
        .and_then(|m| m.as_bool())
        .unwrap_or(false);
    Some((name, description, internal))
}

/// Build a [`Skill`] from its directory.
///
/// The skill name comes from the SKILL.md frontmatter, which must declare a
/// non-empty `name`; the description is optional ([`validated_frontmatter`]).
/// `None` is returned when the directory is not a skill, or when the skill is
/// internal and not allowed (explicit selection passes `allow_internal =
/// true`; otherwise the `INSTALL_INTERNAL_SKILLS=1` opt-in applies).
pub fn read_skill(dir: &Path, allow_internal: bool) -> Option<Skill> {
    let (name, description, internal) = validated_frontmatter(dir)?;
    if internal && !allow_internal && !install_internal_skills() {
        return None;
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
    fn name_and_description_come_from_the_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        // The directory name is irrelevant: the frontmatter declares identity.
        write_skill_md(tmp.path(), "skills/acrobat", "pdf");
        let dir = tmp.path().join("skills/acrobat");
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.name, "pdf");
        assert_eq!(skill.description, "does pdf");
        assert_eq!(skill.dir, dir);
    }

    #[test]
    fn a_directory_is_only_a_skill_with_a_frontmatter_name() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Missing name.
        let dir = tmp.path().join("no-name");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        assert!(read_skill(&dir, false).is_none());

        // Blank name.
        let dir = tmp.path().join("blank-name");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: \"  \"\ndescription: d\n---\nbody",
        )
        .unwrap();
        assert!(read_skill(&dir, false).is_none());

        // No frontmatter, invalid YAML, missing manifest: not skills.
        let dir = tmp.path().join("no-fm");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "# just a heading\n").unwrap();
        assert!(read_skill(&dir, false).is_none());

        let dir = tmp.path().join("bad-yaml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: [unclosed\n---\nbody").unwrap();
        assert!(read_skill(&dir, false).is_none());

        let dir = tmp.path().join("no-md");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_skill(&dir, false).is_none());

        // A missing or blank description is fine: the skill stays valid with
        // an empty description.
        let dir = tmp.path().join("no-desc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\nbody").unwrap();
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.name, "x");
        assert_eq!(skill.description, "");

        let dir = tmp.path().join("blank-desc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: x\ndescription: \"  \"\n---\nbody",
        )
        .unwrap();
        // The description is kept as declared — only the name is validated.
        assert_eq!(read_skill(&dir, false).unwrap().description, "  ");
    }

    #[test]
    fn name_is_trimmed_and_block_description_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("pdf2");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: \" pdf \"\ndescription: |\n  Multi line\n  description here\n---\nbody",
        )
        .unwrap();
        let skill = read_skill(&dir, false).unwrap();
        assert_eq!(skill.name, "pdf");
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
