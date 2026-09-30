//! Source parsing: parse a user-provided source string into a structured [`Source`].
//!
//! Exactly two forms are accepted:
//!
//! - **Local skill directory** — `./my-skill`, `/abs/path/skill`, `C:\skill`:
//!   a directory that directly contains a `SKILL.md` whose frontmatter
//!   declares a non-empty `name` (the description is optional).
//! - **GitHub skill id** — `owner/repo/slug`: one named skill from a GitHub
//!   repository. The first two segments name the hosting repository; the last
//!   segment is the skill's **slug** — its SKILL.md frontmatter `name`
//!   slugified (lowercase, spaces → `-`, `/` dropped, everything else kept
//!   verbatim). The repository tree is searched for the first manifest whose
//!   slugified `name` equals the requested slug; a `SKILL.md` at the
//!   repository root is an ordinary candidate, so a match there installs the
//!   whole repository. The git ref (branch / tag / commit SHA) is orthogonal
//!   to the source string and supplied separately.
//!
//! Everything else is rejected with a hint: bare `owner/repo`, the legacy
//! `owner/repo@<skill>` form, longer repository paths, full GitHub URLs,
//! GitLab / SSH / generic git URLs, and arbitrary HTTPS downloads.

use std::path::{Path, PathBuf};

use crate::error::{Result, SkillsError};

/// The kind of a parsed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// A skill directory on the local filesystem.
    Local,
    /// A skill in a GitHub repository, selected with `owner/repo/slug`.
    Github,
}

/// Parsed source.
///
/// Only the fields meaningful for the [`SourceType`](Source::ty) are populated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Source type.
    pub ty: SourceType,
    /// The original source string.
    pub raw: String,
    /// Absolute directory when the source is local.
    pub local_path: Option<PathBuf>,
    /// GitHub repository owner.
    pub owner: String,
    /// GitHub repository name (without a trailing `.git`).
    pub repo: String,
    /// Skill selector: the skill's slug — the id's last segment.
    pub slug: String,
}

impl Source {
    /// The full `owner/repo/slug` id (GitHub sources only).
    pub fn id(&self) -> String {
        format!("{}/{}/{}", self.owner, self.repo, self.slug)
    }
}

fn is_local_path(input: &str) -> bool {
    let p = Path::new(input);
    if p.is_absolute() {
        return true;
    }
    if input.starts_with("./") || input.starts_with("../") {
        return true;
    }
    if input == "." || input == ".." {
        return true;
    }
    // Windows absolute path, e.g. C:\ or D:/
    let b = input.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

fn unsupported(input: &str, hint: &str) -> SkillsError {
    SkillsError::msg(format!(
        "Unsupported source: \"{input}\". {hint} Supported forms: \
         `owner/repo/slug` (GitHub) or a local skill directory containing SKILL.md."
    ))
}

/// Whether `s` is a usable id segment: non-empty and not a dot path component.
fn valid_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".."
}

/// Parse the GitHub skill id `owner/repo/slug`.
///
/// `Ok(None)` when the input is not an id candidate (fewer than two `/`);
/// every other malformed id is an explicit error — never silently accepted.
fn parse_github_id(input: &str) -> Result<Option<Source>> {
    if input.contains(':') {
        // SSH (`git@host:…`) and anything URL-like with a scheme.
        return Err(unsupported(
            input,
            "SSH and generic git URLs are no longer supported.",
        ));
    }
    if input.starts_with('.') || input.starts_with('/') {
        return Ok(None);
    }
    let segments: Vec<&str> = input.split('/').collect();
    if segments.len() < 2 {
        return Ok(None);
    }
    if segments.len() == 2 {
        let hint = if input.contains('@') {
            "The `owner/repo@<skill>` form is no longer supported; install by \
             id instead: `owner/repo/<slug>`."
        } else {
            "An id has three segments: `owner/repo/<slug>`."
        };
        return Err(unsupported(input, hint));
    }
    if segments.len() > 3 {
        return Err(unsupported(
            input,
            "An id has exactly three segments: `owner/repo/<slug>`.",
        ));
    }
    let owner = segments[0];
    let repo = segments[1];
    let slug = segments[2];
    if !valid_segment(owner) || !valid_segment(repo) || !valid_segment(slug) {
        return Err(unsupported(
            input,
            "Expected `owner/repo/<slug>` — no segment may be empty, `.` or `..`.",
        ));
    }

    Ok(Some(Source {
        ty: SourceType::Github,
        raw: input.to_string(),
        local_path: None,
        owner: owner.to_string(),
        repo: repo.strip_suffix(".git").unwrap_or(repo).to_string(),
        slug: slug.to_string(),
    }))
}

/// Parse a source string (pure function).
pub fn parse_source(input: &str) -> Result<Source> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SkillsError::msg(
            "Empty source. Expected `owner/repo/slug` or a local skill directory.",
        ));
    }

    // Local path: absolute, relative, or current directory.
    if is_local_path(input) {
        let resolved = if Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            std::env::current_dir()?.join(input)
        };
        return Ok(Source {
            ty: SourceType::Local,
            raw: input.to_string(),
            local_path: Some(resolved),
            owner: String::new(),
            repo: String::new(),
            slug: String::new(),
        });
    }

    if input.starts_with("http://") || input.starts_with("https://") {
        return Err(unsupported(
            input,
            "Full URLs and direct HTTPS downloads are no longer supported; \
             pin a version with `--ref <branch|tag|SHA>` if needed.",
        ));
    }

    if let Some(s) = parse_github_id(input)? {
        return Ok(s);
    }

    Err(unsupported(input, "Expected `owner/repo/slug`."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_relative_path() {
        let s = parse_source("./skills/pdf").unwrap();
        assert_eq!(s.ty, SourceType::Local);
        assert!(s.local_path.is_some());
    }

    #[test]
    fn local_absolute_path() {
        let s = parse_source("/tmp/foo").unwrap();
        assert_eq!(s.ty, SourceType::Local);
        assert!(s.local_path.is_some());
    }

    #[test]
    fn local_windows_drive() {
        let s = parse_source(r"C:\foo\skill").unwrap();
        assert_eq!(s.ty, SourceType::Local);
    }

    #[test]
    fn github_id_parses_into_three_segments() {
        let s = parse_source("acme/skills/pdf").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.owner, "acme");
        assert_eq!(s.repo, "skills");
        assert_eq!(s.slug, "pdf");
        assert_eq!(s.id(), "acme/skills/pdf");
    }

    #[test]
    fn github_id_strips_git_suffix() {
        let s = parse_source("acme/skills.git/pdf").unwrap();
        assert_eq!(s.repo, "skills");
    }

    #[test]
    fn slug_keeps_punctuation_verbatim() {
        // `@ & . _` are ordinary slug characters; only the id's segment
        // structure is special (`:` is rejected as URL/SSH-like).
        for slug in ["pdf@v2", "c++.net&a&b", "my_skill"] {
            let s = parse_source(&format!("acme/skills/{slug}")).unwrap();
            assert_eq!(s.slug, slug);
        }
    }

    #[test]
    fn bare_owner_repo_is_rejected() {
        let e = parse_source("acme/skills").unwrap_err();
        assert!(e.to_string().contains("three segments"), "{e}");
        assert!(e.to_string().contains("owner/repo/<slug>"), "{e}");
    }

    #[test]
    fn legacy_at_syntax_is_rejected_with_a_migration_hint() {
        let e = parse_source("acme/skills@pdf").unwrap_err();
        assert!(e.to_string().contains("no longer supported"), "{e}");
        assert!(e.to_string().contains("owner/repo/<slug>"), "{e}");
    }

    #[test]
    fn longer_paths_are_rejected() {
        let e = parse_source("acme/skills/skills/pdf").unwrap_err();
        assert!(e.to_string().contains("exactly three segments"), "{e}");
    }

    #[test]
    fn dot_or_empty_segments_are_rejected() {
        assert!(parse_source("acme//pdf").is_err());
        assert!(parse_source("acme/skills/.").is_err());
        // A leading `..` is a local relative path, not an id.
        let s = parse_source("../skills/pdf").unwrap();
        assert_eq!(s.ty, SourceType::Local);
    }

    #[test]
    fn full_urls_are_rejected() {
        let e = parse_source("https://github.com/acme/skills").unwrap_err();
        assert!(e.to_string().contains("Full URLs"));
        let e = parse_source("https://example.com/skills.zip").unwrap_err();
        assert!(e.to_string().contains("Full URLs"));
    }

    #[test]
    fn gitlab_urls_are_rejected() {
        let e = parse_source("https://gitlab.com/group/repo/-/tree/main/skills/pdf").unwrap_err();
        assert!(e.to_string().contains("Full URLs"));
    }

    #[test]
    fn ssh_git_urls_are_rejected() {
        let e = parse_source("git@github.com:acme/skills.git").unwrap_err();
        assert!(e.to_string().contains("SSH"));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(parse_source("  ").is_err());
    }
}
