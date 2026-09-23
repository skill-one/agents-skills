//! Source parsing: parse a user-provided source string into a structured [`Source`].
//!
//! Exactly two forms are accepted:
//!
//! - **Local skill directory** — `./my-skill`, `/abs/path/skill`, `C:\skill`:
//!   a directory that directly contains a `SKILL.md`.
//! - **GitHub skill** — `owner/repo@<skill>`: one named skill from a GitHub
//!   repository. `<skill>` is a skill **directory name** — a directory
//!   directly containing `SKILL.md`, matched case-insensitively (shallowest
//!   wins); a `SKILL.md` at the repository root is selected with the
//!   repository name. The git ref (branch / tag / commit SHA) is orthogonal
//!   to the source string and supplied separately.
//!
//! Everything else is rejected with a hint: bare `owner/repo`, full GitHub URLs,
//! GitLab / SSH / generic git URLs, and arbitrary HTTPS downloads.

use std::path::{Path, PathBuf};

use crate::error::{Result, SkillsError};

/// The kind of a parsed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// A skill directory on the local filesystem.
    Local,
    /// A skill in a GitHub repository, selected with `owner/repo@<skill>`.
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
    /// Skill selector: the skill's directory name, case-insensitive.
    pub skill: String,
}

impl Source {
    /// `owner/repo` slug for messages (GitHub sources only).
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
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
         `owner/repo@<skill>` (GitHub) or a local skill directory containing SKILL.md."
    ))
}

/// Parse the GitHub shorthand `owner/repo@<skill>`.
///
/// `Ok(None)` when the input is not a shorthand (contains no `/`); every other
/// malformed shorthand is an explicit error — never silently accepted.
fn parse_github_shorthand(input: &str) -> Result<Option<Source>> {
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
    let Some((owner, rest)) = input.split_once('/') else {
        return Ok(None);
    };
    if owner.is_empty() || rest.is_empty() {
        return Err(unsupported(input, "Expected `owner/repo@<skill>`."));
    }
    // A second slash means a repository subpath (with or without `@skill`).
    if rest.contains('/') {
        let repo = rest.split(['/', '@']).next().unwrap_or(rest);
        return Err(SkillsError::msg(format!(
            "Repository subpaths are no longer accepted: \"{input}\". \
             Select the skill by name instead: `{owner}/{repo}@<skill>`."
        )));
    }

    let Some((repo, skill)) = rest.split_once('@') else {
        return Err(SkillsError::msg(format!(
            "Missing skill selector: \"{input}\" installs a whole repository. \
             Name the skill explicitly: `{owner}/{rest}@<skill>`."
        )));
    };
    if repo.is_empty() || skill.is_empty() {
        return Err(unsupported(input, "Expected `owner/repo@<skill>`."));
    }

    Ok(Some(Source {
        ty: SourceType::Github,
        raw: input.to_string(),
        local_path: None,
        owner: owner.to_string(),
        repo: repo.strip_suffix(".git").unwrap_or(repo).to_string(),
        skill: skill.to_string(),
    }))
}

/// Parse a source string (pure function).
pub fn parse_source(input: &str) -> Result<Source> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SkillsError::msg(
            "Empty source. Expected `owner/repo@<skill>` or a local skill directory.",
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
            skill: String::new(),
        });
    }

    if input.starts_with("http://") || input.starts_with("https://") {
        return Err(unsupported(
            input,
            "Full URLs and direct HTTPS downloads are no longer supported; \
             pin a version with `--ref <branch|tag|SHA>` if needed.",
        ));
    }

    if let Some(s) = parse_github_shorthand(input)? {
        return Ok(s);
    }

    Err(unsupported(input, "Expected `owner/repo@<skill>`."))
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
    fn github_shorthand_with_skill() {
        let s = parse_source("acme/skills@pdf").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.owner, "acme");
        assert_eq!(s.repo, "skills");
        assert_eq!(s.skill, "pdf");
        assert_eq!(s.slug(), "acme/skills");
    }

    #[test]
    fn github_shorthand_strips_git_suffix() {
        let s = parse_source("acme/skills.git@pdf").unwrap();
        assert_eq!(s.repo, "skills");
    }

    #[test]
    fn skill_selector_may_contain_at() {
        // Split on the first '@'; a '@' inside the skill name is kept.
        let s = parse_source("acme/skills@pdf@v2").unwrap();
        assert_eq!(s.skill, "pdf@v2");
    }

    #[test]
    fn bare_owner_repo_is_rejected() {
        let e = parse_source("acme/skills").unwrap_err();
        assert!(e.to_string().contains("Missing skill selector"));
        assert!(e.to_string().contains("acme/skills@<skill>"));
    }

    #[test]
    fn subpath_shorthand_is_rejected() {
        let e = parse_source("acme/skills/skills/pdf").unwrap_err();
        assert!(e.to_string().contains("subpaths"));
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
