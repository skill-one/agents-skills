//! Source parsing: parse a user-provided source string into a structured [`Source`].
//!
//! Supports: local paths, GitHub URLs (incl. `/tree/<ref>/<path>`), GitLab URLs
//! (incl. `/-/tree/`), GitHub shorthand (`owner/repo`, `owner/repo@skill`,
//! `owner/repo/subpath`), SSH / generic git URLs, and arbitrary https (direct
//! download + extract).
//!
//! Not supported: `github:`/`gitlab:` prefixes, `#ref@skill` fragments,
//! SOURCE_ALIASES alias mapping, and GitHub URL paths without `/tree/` (bare
//! paths and `/blob/` URLs are rejected with a hint).

use std::path::{Path, PathBuf};

use url::Url;

use crate::error::{Result, SkillsError};

/// The kind of a parsed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// GitHub repository (has an API/blob fast path).
    Github,
    /// GitLab repository.
    Gitlab,
    /// Generic git repository (SSH / any git host).
    Git,
    /// Local filesystem path.
    Local,
    /// Hosted artifact or arbitrary https endpoint, downloaded and extracted
    /// directly (raw file / archive / release asset / any non-git https URL).
    Download,
}

/// Parsed source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Source type.
    pub ty: SourceType,
    /// Primary URL for install/download (resolved absolute path for local sources).
    pub url: String,
    /// Subpath within the repository (e.g. `skills/pdf`).
    pub subpath: Option<String>,
    /// Absolute path when the source is local.
    pub local_path: Option<PathBuf>,
    /// Branch or tag ref.
    pub r#ref: Option<String>,
    /// Skill name selected via the `@skill` syntax.
    pub skill_filter: Option<String>,
}

impl Source {
    fn new(ty: SourceType, url: impl Into<String>) -> Self {
        Source {
            ty,
            url: url.into(),
            subpath: None,
            local_path: None,
            r#ref: None,
            skill_filter: None,
        }
    }

    /// Whole-repo archive URL for hosts that publish tarballs without a git clone
    /// (GitHub codeload, GitLab). `None` when the archive would need a ref we cannot
    /// resolve (e.g. GitLab without an explicit branch/tag).
    ///
    /// `tar.gz` is deliberate: it keeps the Unix mode of every entry, so executable
    /// bits survive the round trip (zip archives drop them).
    pub fn archive_url(&self) -> Option<String> {
        match self.ty {
            SourceType::Github => {
                // `HEAD` resolves to the default branch on codeload.
                let r = self.r#ref.clone().unwrap_or_else(|| "HEAD".to_string());
                Some(format!(
                    "https://codeload.github.com/{}/tar.gz/{r}",
                    owner_repo(&self.url)
                ))
            }
            SourceType::Gitlab => {
                let r = self.r#ref.clone()?;
                let base = self.url.trim_end_matches(".git");
                Some(format!("{base}/-/archive/{r}/{r}.tar.gz"))
            }
            _ => None,
        }
    }
}

/// Reject subpaths containing `..` segments to prevent path traversal.
pub fn sanitize_subpath(subpath: &str) -> Result<String> {
    let normalized = subpath.replace('\\', "/");
    if normalized.split('/').any(|seg| seg == "..") {
        return Err(SkillsError::msg(format!(
            "Unsafe subpath: \"{subpath}\" contains path traversal segments. Subpaths must not contain \"..\" components."
        )));
    }
    Ok(subpath.to_string())
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

/// Hosted artifact direct links (raw/archive/release asset) must be downloaded directly,
/// not normalized into a parent repo clone.
fn is_hosted_artifact_url(input: &str) -> bool {
    let Ok(parsed) = Url::parse(input) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    if matches!(
        host.as_str(),
        "raw.githubusercontent.com" | "codeload.github.com" | "objects.githubusercontent.com"
    ) {
        return true;
    }
    let path = parsed.path();
    if host == "github.com" {
        // /<owner>/<repo>/archive/... | /raw/... | /releases/download/... | /releases/latest/download/...
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 4 {
            return false;
        }
        let kind = segs[2];
        if kind == "archive" || kind == "raw" {
            return true;
        }
        if kind == "releases" {
            return segs[3] == "download"
                || (segs[3] == "latest" && segs.get(4) == Some(&"download"));
        }
        return false;
    }
    if host == "gitlab.com" {
        return path.contains("/-/archive/") || path.contains("/-/raw/");
    }
    false
}

/// URL parsers auto-normalize `..` segments, so we must pre-check the raw input for traversal.
fn reject_traversal(input: &str) -> Result<()> {
    sanitize_subpath(input).map(|_| ())
}

fn parse_github_url(input: &str) -> Result<Option<Source>> {
    reject_traversal(input)?;
    let Ok(parsed) = Url::parse(input) else {
        return Ok(None);
    };
    if parsed.host_str() != Some("github.com") {
        return Ok(None);
    }
    let segs: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() < 2 {
        return Ok(None);
    }
    // A URL path is only meaningful through /tree/<ref>[/<path>]; anything else
    // (bare path, /blob/) would silently install the wrong scope.
    if segs.len() > 2 && segs[2] != "tree" {
        let hint = if segs[2] == "blob" {
            "GitHub /blob/ URLs point at a single file; use /tree/<ref>/<path>, owner/repo[/subpath], or owner/repo@skill instead"
        } else {
            "use /tree/<ref>/<path>, owner/repo[/subpath], or owner/repo@skill to select a subpath"
        };
        return Err(SkillsError::msg(format!(
            "Unsupported GitHub URL: \"{input}\". {hint}."
        )));
    }
    let repo = segs[1].strip_suffix(".git").unwrap_or(segs[1]);
    let mut s = Source::new(
        SourceType::Github,
        format!("https://github.com/{}/{}.git", segs[0], repo),
    );
    // /tree/<ref>[/<subpath>]
    if segs.get(2) == Some(&"tree") && segs.get(3).is_some() {
        s.r#ref = Some(segs[3].to_string());
        if segs.len() > 4 {
            s.subpath = Some(sanitize_subpath(&segs[4..].join("/"))?);
        }
    }
    Ok(Some(s))
}

fn parse_gitlab_url(input: &str) -> Result<Option<Source>> {
    reject_traversal(input)?;
    let Ok(parsed) = Url::parse(input) else {
        return Ok(None);
    };
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let path = parsed.path();

    // GitLab /-/blob/ points at a single file and would otherwise be mangled
    // into a bogus repo path; reject it with a hint instead.
    if path.contains("/-/blob/") {
        return Err(SkillsError::msg(format!(
            "Unsupported GitLab URL: \"{input}\". GitLab /blob/ URLs point at a single file; use /-/tree/<ref>/<path> or the repo URL instead."
        )));
    }

    // Any GitLab instance's /-/tree/<ref>[/<subpath>]
    if let Some(idx) = path.find("/-/tree/") {
        if host == "github.com" {
            return Ok(None);
        }
        let repo_path = path[..idx].strip_suffix(".git").unwrap_or(&path[..idx]);
        let rest = &path[idx + "/-/tree/".len()..];
        let (ref_part, subpath) = match rest.split_once('/') {
            Some((r, sp)) => (r, Some(sp.to_string())),
            None => (rest, None),
        };
        let mut s = Source::new(SourceType::Gitlab, format!("https://{host}{repo_path}.git"));
        s.r#ref = Some(ref_part.to_string());
        if let Some(sp) = subpath {
            s.subpath = Some(sanitize_subpath(&sp)?);
        }
        return Ok(Some(s));
    }

    // gitlab.com/<group>/<subgroup>/<repo>
    if host == "gitlab.com" {
        let trimmed = path.trim_end_matches('/').strip_prefix('/').unwrap_or(path);
        let repo_path = trimmed.strip_suffix(".git").unwrap_or(trimmed);
        if repo_path.contains('/') {
            return Ok(Some(Source::new(
                SourceType::Gitlab,
                format!("https://gitlab.com/{repo_path}.git"),
            )));
        }
    }
    Ok(None)
}

/// GitHub shorthand: `owner/repo`, `owner/repo@skill`, `owner/repo/subpath`.
fn parse_shorthand(input: &str) -> Result<Option<Source>> {
    if input.contains(':') || input.starts_with('.') || input.starts_with('/') {
        return Ok(None);
    }
    let (owner, rest) = match input.split_once('/') {
        Some(v) => v,
        None => return Ok(None),
    };
    if owner.is_empty() || rest.is_empty() {
        return Ok(None);
    }

    // owner/repo@skill (repo has no / or @)
    if let Some((repo, skill)) = rest.split_once('@')
        && !repo.is_empty()
        && !repo.contains('/')
        && !repo.contains('@')
        && !skill.is_empty()
    {
        let mut s = Source::new(
            SourceType::Github,
            format!("https://github.com/{owner}/{repo}.git"),
        );
        s.skill_filter = Some(skill.to_string());
        return Ok(Some(s));
    }

    // owner/repo[/subpath]
    // An '@' surviving this far means a subpath and @skill were combined
    // (e.g. owner/repo/skills/pdf@pdf); reject it instead of swallowing the
    // '@' into the subpath.
    if rest.contains('@') {
        return Err(SkillsError::msg(format!(
            "Cannot combine a subpath with @skill selection in \"{input}\"; use --skill to pick a skill."
        )));
    }
    let segs: Vec<&str> = rest.split('/').collect();
    let repo = segs[0];
    if repo.is_empty() {
        return Ok(None);
    }
    let mut s = Source::new(
        SourceType::Github,
        format!("https://github.com/{owner}/{repo}.git"),
    );
    let subpath = segs[1..].join("/").trim_end_matches('/').to_string();
    if !subpath.is_empty() {
        s.subpath = Some(sanitize_subpath(&subpath)?);
    }
    Ok(Some(s))
}

/// Parse a source string (pure function).
pub fn parse_source(input: &str) -> Result<Source> {
    // Local path: absolute, relative, or current directory.
    if is_local_path(input) {
        let resolved = if Path::new(input).is_absolute() {
            PathBuf::from(input)
        } else {
            std::env::current_dir()?.join(input)
        };
        return Ok(Source {
            ty: SourceType::Local,
            url: resolved.to_string_lossy().into_owned(),
            local_path: Some(resolved),
            subpath: None,
            r#ref: None,
            skill_filter: None,
        });
    }

    if is_hosted_artifact_url(input) {
        return Ok(Source::new(SourceType::Download, input));
    }
    if let Some(s) = parse_github_url(input)? {
        return Ok(s);
    }
    if let Some(s) = parse_gitlab_url(input)? {
        return Ok(s);
    }
    if let Some(s) = parse_shorthand(input)? {
        return Ok(s);
    }
    // Any other https/http endpoint: download + extract (zip / tar / single file).
    if input.starts_with("http://") || input.starts_with("https://") {
        if input.ends_with(".git") {
            return Ok(Source::new(SourceType::Git, input));
        }
        return Ok(Source::new(SourceType::Download, input));
    }

    // Fallback: treat as a generic git URL.
    Ok(Source::new(SourceType::Git, input))
}

/// Extract `owner/repo` from `https://github.com/<owner>/<repo>.git`.
pub fn owner_repo(url: &str) -> String {
    let trimmed = url.trim_end_matches(".git");
    let parts = trimmed.split('/').filter(|s| !s.is_empty());
    let mut owner = "";
    let mut repo = "";
    let mut found_host = false;
    for p in parts {
        if p == "github.com" {
            found_host = true;
            continue;
        }
        if found_host {
            if owner.is_empty() {
                owner = p;
            } else if repo.is_empty() {
                repo = p;
                break;
            }
        }
    }
    if owner.is_empty() {
        url.to_string()
    } else if repo.is_empty() {
        owner.to_string()
    } else {
        format!("{owner}/{repo}")
    }
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
        assert!(s.url.starts_with('/'));
    }

    #[test]
    fn local_windows_drive() {
        let s = parse_source(r"C:\foo\skill").unwrap();
        assert_eq!(s.ty, SourceType::Local);
    }

    #[test]
    fn github_shorthand() {
        let s = parse_source("acme/skills").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.url, "https://github.com/acme/skills.git");
        assert_eq!(s.subpath, None);
        assert_eq!(s.r#ref, None);
        assert_eq!(s.skill_filter, None);
    }

    #[test]
    fn github_shorthand_with_skill() {
        let s = parse_source("acme/skills@pdf").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.skill_filter.as_deref(), Some("pdf"));
    }

    #[test]
    fn github_shorthand_with_subpath() {
        let s = parse_source("acme/skills/skills/pdf").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.subpath.as_deref(), Some("skills/pdf"));
    }

    #[test]
    fn github_url() {
        let s = parse_source("https://github.com/acme/skills").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.url, "https://github.com/acme/skills.git");
    }

    #[test]
    fn github_url_with_git_suffix() {
        let s = parse_source("https://github.com/acme/skills.git").unwrap();
        assert_eq!(s.ty, SourceType::Github);
        assert_eq!(s.url, "https://github.com/acme/skills.git");
    }

    #[test]
    fn github_url_tree_branch() {
        let s = parse_source("https://github.com/acme/skills/tree/main").unwrap();
        assert_eq!(s.r#ref.as_deref(), Some("main"));
        assert_eq!(s.subpath, None);
    }

    #[test]
    fn github_url_tree_with_subpath() {
        let s = parse_source("https://github.com/acme/skills/tree/main/skills/pdf").unwrap();
        assert_eq!(s.r#ref.as_deref(), Some("main"));
        assert_eq!(s.subpath.as_deref(), Some("skills/pdf"));
    }

    #[test]
    fn github_bare_path_url_is_rejected() {
        let e = parse_source("https://github.com/acme/skills/skills/pdf").unwrap_err();
        assert!(e.to_string().contains("Unsupported GitHub URL"));
    }

    #[test]
    fn github_blob_url_is_rejected() {
        let e = parse_source("https://github.com/acme/skills/blob/main/skills/pdf/SKILL.md")
            .unwrap_err();
        assert!(e.to_string().contains("/blob/"));
    }

    #[test]
    fn gitlab_blob_url_is_rejected() {
        let e = parse_source("https://gitlab.com/group/repo/-/blob/main/skills/pdf/SKILL.md")
            .unwrap_err();
        assert!(e.to_string().contains("/blob/"));
    }

    #[test]
    fn shorthand_subpath_with_skill_is_rejected() {
        let e = parse_source("acme/skills/skills/pdf@pdf").unwrap_err();
        assert!(e.to_string().contains("--skill"));
    }

    #[test]
    fn gitlab_url_with_subgroups() {
        let s = parse_source("https://gitlab.com/group/subgroup/repo").unwrap();
        assert_eq!(s.ty, SourceType::Gitlab);
        assert_eq!(s.url, "https://gitlab.com/group/subgroup/repo.git");
    }

    #[test]
    fn gitlab_tree_with_subpath() {
        let s = parse_source("https://gitlab.com/group/repo/-/tree/main/skills/pdf").unwrap();
        assert_eq!(s.ty, SourceType::Gitlab);
        assert_eq!(s.r#ref.as_deref(), Some("main"));
        assert_eq!(s.subpath.as_deref(), Some("skills/pdf"));
    }

    #[test]
    fn ssh_git_url() {
        let s = parse_source("git@github.com:acme/skills.git").unwrap();
        assert_eq!(s.ty, SourceType::Git);
        assert_eq!(s.url, "git@github.com:acme/skills.git");
    }

    #[test]
    fn https_url_is_download() {
        let s = parse_source("https://example.com/foo/skill").unwrap();
        assert_eq!(s.ty, SourceType::Download);
    }

    #[test]
    fn url_ending_with_git_is_git() {
        let s = parse_source("https://example.com/foo.git").unwrap();
        assert_eq!(s.ty, SourceType::Git);
    }

    #[test]
    fn raw_github_artifact_is_download() {
        let s = parse_source("https://raw.githubusercontent.com/x/y/main/SKILL.md").unwrap();
        assert_eq!(s.ty, SourceType::Download);
    }

    #[test]
    fn unsafe_subpath_is_rejected() {
        assert!(parse_source("acme/skills/a/../b").is_err());
        assert!(parse_source("https://github.com/x/y/tree/main/a/../b").is_err());
    }

    #[test]
    fn archive_url_github_default_branch() {
        let s = parse_source("acme/skills").unwrap();
        assert_eq!(
            s.archive_url().as_deref(),
            Some("https://codeload.github.com/acme/skills/tar.gz/HEAD")
        );
    }

    #[test]
    fn archive_url_github_with_ref() {
        let s = parse_source("https://github.com/acme/skills/tree/main").unwrap();
        assert_eq!(
            s.archive_url().as_deref(),
            Some("https://codeload.github.com/acme/skills/tar.gz/main")
        );
    }

    #[test]
    fn archive_url_gitlab_with_ref() {
        let s = parse_source("https://gitlab.com/group/sub/repo/-/tree/main").unwrap();
        assert_eq!(
            s.archive_url().as_deref(),
            Some("https://gitlab.com/group/sub/repo/-/archive/main/main.tar.gz")
        );
    }

    #[test]
    fn archive_url_none_without_ref_or_for_other_types() {
        // GitLab without a ref cannot resolve a default branch for the archive.
        let s = parse_source("https://gitlab.com/group/sub/repo").unwrap();
        assert_eq!(s.archive_url(), None);
        // Download sources do not use archive URLs.
        let s = parse_source("https://example.com/x.zip").unwrap();
        assert_eq!(s.archive_url(), None);
    }
}
