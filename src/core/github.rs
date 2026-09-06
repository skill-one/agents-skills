//! GitHub API fast path: fetch only what is needed instead of the whole
//! repository archive — the directory of a `@skill`-selected skill, or every
//! file under a subpath.
//!
//! The HTTP client is injected as a `get` closure so the whole selection logic is
//! unit-testable offline; production uses a plain ureq GET.

use std::io::Read;
use std::path::PathBuf;

use serde_json::Value;

use crate::core::discover::{Skill, filter_skills, parse_skill_md_inner};
use crate::core::source::{Source, SourceType, owner_repo};
use crate::error::{Result, SkillsError};

/// Fetch the files of the `@skill`-selected skill dir into a fresh temp dir.
///
/// - `Ok(Some((temp, root)))`: the skill was found; `root` holds it at its original
///   relative path (e.g. `root/pdf/SKILL.md`).
/// - `Ok(None)`: the GitHub API worked but no skill matched the name.
/// - `Err`: API/network failure — callers should fall back to a full archive fetch.
pub fn fetch_skill_via_api(
    parsed: &Source,
    skill_name: &str,
    include_internal: bool,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    fetch_skill_via_api_with(parsed, skill_name, include_internal, &http_get)
}

/// Fetch only the files under `parsed.subpath` into a fresh temp dir, keeping
/// their original relative paths so discovery can run on the root as usual.
///
/// - `Ok(Some((temp, root)))`: the subpath files were fetched.
/// - `Ok(None)`: the GitHub API worked but nothing exists under the subpath.
/// - `Err`: API/network failure — callers should fall back to a full archive fetch.
pub fn fetch_subdir_via_api(parsed: &Source) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    fetch_subdir_via_api_with(parsed, &http_get)
}

fn fetch_subdir_via_api_with(
    parsed: &Source,
    get: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    let Some(subpath) = parsed.subpath.as_deref() else {
        return Ok(None);
    };
    let Some(tree) = fetch_tree_entries_with(parsed, get)? else {
        return Ok(None);
    };
    download_blobs_under(&tree, Some(subpath), get)
}

/// A resolved ref plus the recursive tree listing of the repository.
struct RepoTree {
    owner: String,
    repo: String,
    r#ref: String,
    /// Every path in the repo as `(path, type)` (`blob` / `tree`).
    entries: Vec<(String, String)>,
}

/// Resolve the ref (explicit, or the repository's default branch) and list the
/// whole repo tree recursively. `Ok(None)` when the source is not GitHub.
fn fetch_tree_entries_with(
    parsed: &Source,
    get: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Option<RepoTree>> {
    if parsed.ty != SourceType::Github {
        return Ok(None);
    }
    let owner_repo = owner_repo(&parsed.url);
    let (owner, repo) = owner_repo
        .split_once('/')
        .unwrap_or((owner_repo.as_str(), ""));

    // Resolve the ref: an explicit branch/tag, or the repository's default branch.
    let r#ref = match &parsed.r#ref {
        Some(r) => r.clone(),
        None => {
            let body = get(&repo_url(owner, repo))?;
            let v: Value = serde_json::from_slice(&body)?;
            v.get("default_branch")
                .and_then(|b| b.as_str())
                .map(str::to_string)
                .ok_or_else(|| SkillsError::msg("GitHub API: missing default_branch"))?
        }
    };

    // Recursive tree listing gives every path in the repo in one call.
    let body = get(&tree_url(owner, repo, &r#ref)?)?;
    let v: Value = serde_json::from_slice(&body)?;
    if v.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Err(SkillsError::msg("GitHub tree listing truncated"));
    }
    let entries: Vec<(String, String)> = v
        .get("tree")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    Some((
                        e.get("path")?.as_str()?.to_string(),
                        e.get("type")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(RepoTree {
        owner: owner.to_string(),
        repo: repo.to_string(),
        r#ref,
        entries,
    }))
}

/// Download every blob under `prefix` (all blobs when `prefix` is None) into a
/// fresh temp dir at their original relative paths. `Ok(None)` when nothing
/// matches, so callers can fall back to a full fetch.
fn download_blobs_under(
    tree: &RepoTree,
    prefix: Option<&str>,
    get: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    let files: Vec<String> = match prefix {
        None => tree
            .entries
            .iter()
            .filter(|(_, t)| t == "blob")
            .map(|(p, _)| p.clone())
            .collect(),
        Some(p) => {
            let pref = format!("{p}/");
            tree.entries
                .iter()
                .filter(|(path, t)| t == "blob" && path.starts_with(&pref))
                .map(|(p, _)| p.clone())
                .collect()
        }
    };
    if files.is_empty() {
        return Ok(None);
    }

    let out = tempfile::TempDir::new()?;
    for f in &files {
        let url = raw_url(&tree.owner, &tree.repo, &tree.r#ref, f)?;
        let bytes = get(&url)?;
        let target = out.path().join(f);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    let root = out.path().to_path_buf();
    Ok(Some((out, root)))
}

/// Real HTTP GET used by default (injectable for tests).
///
/// Uses the shared proxy-aware agent, honors `GITHUB_TOKEN` to raise the API rate
/// limit when set, and retries transient failures with backoff.
fn http_get(url: &str) -> Result<Vec<u8>> {
    let attempt = || -> Result<Vec<u8>> {
        let mut req = crate::core::fetch::agent()
            .get(url)
            .set("User-Agent", "agents-skills");
        if let Ok(tok) = std::env::var("GITHUB_TOKEN")
            && !tok.is_empty()
        {
            req = req.set("Authorization", &format!("Bearer {tok}"));
        }
        let resp = req.call()?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf)?;
        Ok(buf)
    };
    crate::core::fetch::with_retry(3, attempt)
}

fn fetch_skill_via_api_with(
    parsed: &Source,
    skill_name: &str,
    include_internal: bool,
    get: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    let Some(tree) = fetch_tree_entries_with(parsed, get)? else {
        return Ok(None);
    };

    // Candidate skill dirs (parents of any `SKILL.md`), shallowest first so the
    // first name/dir match shadows deeper ones (mirrors discover's priority).
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for (path, ty) in &tree.entries {
        if ty != "blob" {
            continue;
        }
        let dir = path.strip_suffix("/SKILL.md").unwrap_or_default();
        if path == "SKILL.md" || path.ends_with("/SKILL.md") {
            let depth = if dir.is_empty() {
                0
            } else {
                dir.matches('/').count() + 1
            };
            candidates.push((depth, dir.to_string()));
        }
    }
    candidates.sort_by_key(|(d, _)| *d);
    candidates.dedup_by(|a, b| a.1 == b.1);

    // Download each candidate SKILL.md (in order) until one matches the name.
    let scratch = tempfile::TempDir::new()?;
    let mut matched: Option<Skill> = None;
    'outer: for (_, dir) in &candidates {
        let url = raw_url(
            &tree.owner,
            &tree.repo,
            &tree.r#ref,
            &format!("{dir}/SKILL.md"),
        )?;
        let Ok(bytes) = get(&url) else { continue };
        let md = scratch.path().join(dir.replace('/', "_")).join("SKILL.md");
        if let Some(parent) = md.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&md, bytes)?;
        if let Some(mut skill) = parse_skill_md_inner(&md, include_internal) {
            skill.dir = PathBuf::from(dir);
            if !filter_skills(std::slice::from_ref(&skill), &[skill_name.to_string()]).is_empty() {
                matched = Some(skill);
                break 'outer;
            }
        }
    }

    let Some(skill) = matched else {
        return Ok(None);
    };
    let dir = skill.dir.to_string_lossy().into_owned();
    let prefix = if dir.is_empty() {
        None
    } else {
        Some(dir.as_str())
    };
    download_blobs_under(&tree, prefix, get)
}

/// `https://api.github.com/repos/{owner}/{repo}` (default branch lookup).
fn repo_url(owner: &str, repo: &str) -> String {
    format!("https://api.github.com/repos/{owner}/{repo}")
}

/// `https://api.github.com/repos/{owner}/{repo}/git/trees/{ref}?recursive=1`.
fn tree_url(owner: &str, repo: &str, r#ref: &str) -> Result<String> {
    let mut u = url::Url::parse("https://api.github.com").expect("valid base URL");
    u.path_segments_mut()
        .map_err(|_| SkillsError::msg("invalid GitHub API URL"))?
        .push("repos")
        .push(owner)
        .push(repo)
        .push("git")
        .push("trees")
        .extend(r#ref.split('/'));
    u.set_query(Some("recursive=1"));
    Ok(u.to_string())
}

/// `https://raw.githubusercontent.com/{owner}/{repo}/{ref}/{path...}`.
fn raw_url(owner: &str, repo: &str, r#ref: &str, path: &str) -> Result<String> {
    let mut u = url::Url::parse("https://raw.githubusercontent.com").expect("valid base URL");
    u.path_segments_mut()
        .map_err(|_| SkillsError::msg("invalid raw URL"))?
        .push(owner)
        .push(repo)
        .extend(r#ref.split('/'))
        .extend(path.split('/'));
    Ok(u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::source::parse_source;

    /// Build an injected `get` serving a tiny in-memory GitHub repo.
    fn fake_get(
        default_branch: &'static str,
        tree: &[(&'static str, &'static str)],
        files: &[(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Result<Vec<u8>> {
        move |url: &str| {
            if url.contains("/git/trees/") {
                let entries: Vec<Value> = tree
                    .iter()
                    .map(|(p, t)| serde_json::json!({"path": p, "type": t}))
                    .collect();
                return Ok(serde_json::to_vec(&serde_json::json!({
                    "tree": entries,
                    "truncated": false
                }))
                .unwrap());
            }
            if url.contains("/repos/acme/skills") {
                return Ok(serde_json::to_vec(&serde_json::json!({
                    "default_branch": default_branch
                }))
                .unwrap());
            }
            for (p, content) in files {
                if url.ends_with(&format!("/{p}")) {
                    return Ok(content.as_bytes().to_vec());
                }
            }
            Err(SkillsError::msg(format!("unexpected url: {url}")))
        }
    }

    #[test]
    fn fetch_via_api_finds_skill_dir_only() {
        let parsed = parse_source("acme/skills@pdf").unwrap();
        let tree = [
            ("pdf/SKILL.md", "blob"),
            ("pdf/scripts/run.sh", "blob"),
            ("skills/doc/SKILL.md", "blob"),
            ("README.md", "blob"),
        ];
        let files = [
            ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
            ("pdf/scripts/run.sh", "#!/bin/sh\n"),
            (
                "skills/doc/SKILL.md",
                "---\nname: doc\ndescription: d\n---\nbody",
            ),
        ];
        let get = fake_get("main", &tree, &files);
        let (tmp, root) = fetch_skill_via_api_with(&parsed, "pdf", true, &get)
            .unwrap()
            .expect("pdf should match");

        assert!(root.join("pdf/SKILL.md").is_file());
        assert!(root.join("pdf/scripts/run.sh").is_file());
        // Only the matched skill dir is fetched.
        assert!(!root.join("skills/doc/SKILL.md").exists());
        assert!(!root.join("README.md").exists());
        let _ = tmp;
    }

    #[test]
    fn fetch_via_api_no_match_returns_none() {
        let parsed = parse_source("acme/skills@zzz").unwrap();
        let tree = [("pdf/SKILL.md", "blob"), ("skills/doc/SKILL.md", "blob")];
        let files = [
            ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
            (
                "skills/doc/SKILL.md",
                "---\nname: doc\ndescription: d\n---\nbody",
            ),
        ];
        let get = fake_get("main", &tree, &files);
        let res = fetch_skill_via_api_with(&parsed, "zzz", true, &get).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn fetch_via_api_api_error_propagates() {
        let parsed = parse_source("acme/skills@pdf").unwrap();
        let get = |url: &str| -> Result<Vec<u8>> {
            Err(SkillsError::msg(format!("network down: {url}")))
        };
        assert!(fetch_skill_via_api_with(&parsed, "pdf", true, &get).is_err());
    }

    #[test]
    fn fetch_via_api_ignores_non_github() {
        let parsed = parse_source("https://gitlab.com/acme/skills/-/tree/main").unwrap();
        let get = |_: &str| -> Result<Vec<u8>> { unreachable!("no HTTP for non-github") };
        assert!(
            fetch_skill_via_api_with(&parsed, "pdf", true, &get)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fetch_subdir_downloads_only_that_dir() {
        let parsed = parse_source("acme/skills/skills/pdf").unwrap();
        let tree = [
            ("skills/pdf/SKILL.md", "blob"),
            ("skills/pdf/scripts/run.sh", "blob"),
            ("skills/doc/SKILL.md", "blob"),
            ("README.md", "blob"),
        ];
        let files = [
            (
                "skills/pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
            ),
            ("skills/pdf/scripts/run.sh", "#!/bin/sh\n"),
            (
                "skills/doc/SKILL.md",
                "---\nname: doc\ndescription: d\n---\nbody",
            ),
            ("README.md", "# read me"),
        ];
        let get = fake_get("main", &tree, &files);
        let (tmp, root) = fetch_subdir_via_api_with(&parsed, &get)
            .unwrap()
            .expect("files under skills/pdf should match");

        assert!(root.join("skills/pdf/SKILL.md").is_file());
        assert!(root.join("skills/pdf/scripts/run.sh").is_file());
        // Files outside the subpath are not fetched.
        assert!(!root.join("skills/doc/SKILL.md").exists());
        assert!(!root.join("README.md").exists());
        let _ = tmp;
    }

    #[test]
    fn fetch_subdir_without_files_returns_none() {
        let parsed = parse_source("acme/skills/skills/missing").unwrap();
        let tree = [("skills/pdf/SKILL.md", "blob"), ("README.md", "blob")];
        let files = [(
            "skills/pdf/SKILL.md",
            "---\nname: pdf\ndescription: d\n---\nbody",
        )];
        let get = fake_get("main", &tree, &files);
        let res = fetch_subdir_via_api_with(&parsed, &get).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn fetch_subdir_ignores_non_github() {
        let parsed = parse_source("https://gitlab.com/acme/skills/-/tree/main/skills/pdf").unwrap();
        let get = |_: &str| -> Result<Vec<u8>> { unreachable!("no HTTP for non-github") };
        assert!(fetch_subdir_via_api_with(&parsed, &get).unwrap().is_none());
    }

    #[test]
    fn raw_and_tree_urls_are_encoded() {
        assert_eq!(
            raw_url("o", "r", "feat/x", "skills/a b/SKILL.md").unwrap(),
            "https://raw.githubusercontent.com/o/r/feat/x/skills/a%20b/SKILL.md"
        );
        assert_eq!(
            tree_url("o", "r", "feat/x").unwrap(),
            "https://api.github.com/repos/o/r/git/trees/feat/x?recursive=1"
        );
    }
}
