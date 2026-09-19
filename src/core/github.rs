//! GitHub API fast path: fetch only what is needed instead of the whole
//! repository archive — the directory of a `@skill`-selected skill, or every
//! file under a subpath.
//!
//! Listing is one recursive `git/trees` call. When GitHub truncates that tree
//! (large repos) we switch to per-directory `contents` calls — the workaround
//! the official docs recommend — instead of failing or downloading the whole
//! repository. Files are then fetched concurrently from
//! `raw.githubusercontent.com`, Git LFS pointers are resolved through
//! `media.githubusercontent.com`, and the git mode from the tree listing
//! restores the executable bit that zip archives lose.
//!
//! The HTTP client is injected as a `get` closure so the whole selection logic is
//! unit-testable offline; production uses a plain ureq GET.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::core::discover::{filter_skills, parse_skill_md_inner};
use crate::core::source::{Source, SourceType, owner_repo};
use crate::error::{Result, SkillsError};

/// Injected HTTP GET: returns the response body of `url`.
type Get<'a> = &'a (dyn Fn(&str) -> Result<Vec<u8>> + Sync);

/// How many files are fetched in parallel.
const DOWNLOAD_CONCURRENCY: usize = 8;

/// A Git LFS pointer is a short text stub; the real object lives elsewhere.
const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

/// One file to fetch, plus the git mode used to restore the executable bit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteFile {
    /// Path relative to the repository root.
    path: String,
    /// Git mode (`100755` = executable); `None` when the listing API omits it.
    mode: Option<String>,
}

/// Resolved repository coordinates, shared by every listing/download call.
#[derive(Debug, Clone)]
struct RepoRef {
    owner: String,
    repo: String,
    r#ref: String,
}

impl RepoRef {
    /// Resolve the ref (explicit, or the repository's default branch).
    ///
    /// `Ok(None)` when the source is not GitHub.
    fn resolve(parsed: &Source, get: Get) -> Result<Option<Self>> {
        if parsed.ty != SourceType::Github {
            return Ok(None);
        }
        let owner_repo = owner_repo(&parsed.url);
        let (owner, repo) = owner_repo
            .split_once('/')
            .unwrap_or((owner_repo.as_str(), ""));

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

        Ok(Some(RepoRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            r#ref,
        }))
    }
}

/// Fetch the files of the `@skill`-selected skill dir into a fresh temp dir.
///
/// - `Ok(Some((temp, root)))`: the skill was found; `root` holds it at its original
///   relative path (e.g. `root/pdf/SKILL.md`).
/// - `Ok(None)`: the GitHub API worked but no skill matched the name.
/// - `Err`: API/network failure — reported to the user; callers must not widen the
///   request to a whole-repo archive fetch.
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
/// This is what keeps a subpath install from downloading (and keeping) the whole
/// repository, so it must never silently widen to the repository root.
///
/// - `Ok(Some((temp, root)))`: the subpath files were fetched.
/// - `Ok(None)`: the GitHub API worked but nothing exists under the subpath.
/// - `Err`: API/network failure — reported to the user; callers must not widen the
///   request to a whole-repo archive fetch.
pub fn fetch_subdir_via_api(parsed: &Source) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    fetch_subdir_via_api_with(parsed, &http_get)
}

fn fetch_subdir_via_api_with(
    parsed: &Source,
    get: Get,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    let Some(subpath) = parsed.subpath.as_deref() else {
        return Ok(None);
    };
    let Some(rr) = RepoRef::resolve(parsed, get)? else {
        return Ok(None);
    };
    let files = list_files_under(&rr, Some(subpath), get)?;
    if files.is_empty() {
        return Ok(None);
    }
    download_files(&rr, &files, get).map(Some)
}

fn fetch_skill_via_api_with(
    parsed: &Source,
    skill_name: &str,
    include_internal: bool,
    get: Get,
) -> Result<Option<(tempfile::TempDir, PathBuf)>> {
    let Some(rr) = RepoRef::resolve(parsed, get)? else {
        return Ok(None);
    };
    let files = list_files_under(&rr, parsed.subpath.as_deref(), get)?;

    // Candidate skill dirs (parents of any `SKILL.md`), shallowest first so a
    // name match shadows deeper ones (mirrors discover's priority).
    let mut candidates: Vec<(usize, String)> = files
        .iter()
        .filter_map(|f| {
            let dir = match f.path.as_str() {
                "SKILL.md" => "",
                p => p.strip_suffix("/SKILL.md")?,
            };
            let depth = if dir.is_empty() {
                0
            } else {
                dir.matches('/').count() + 1
            };
            Some((depth, dir.to_string()))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    candidates.dedup_by(|a, b| a.1 == b.1);

    // Fetch every candidate manifest in one batch, then take the first match in
    // depth order.
    let manifests: Vec<RemoteFile> = candidates
        .iter()
        .map(|(_, dir)| RemoteFile {
            path: if dir.is_empty() {
                "SKILL.md".to_string()
            } else {
                format!("{dir}/SKILL.md")
            },
            mode: None,
        })
        .collect();
    let (_scratch, scratch_root) = download_files(&rr, &manifests, get)?;

    let mut matched: Option<String> = None;
    for (i, (_, dir)) in candidates.iter().enumerate() {
        let md = scratch_root.join(&manifests[i].path);
        if let Some(skill) = parse_skill_md_inner(&md, include_internal)
            && !filter_skills(std::slice::from_ref(&skill), &[skill_name.to_string()]).is_empty()
        {
            matched = Some(dir.clone());
            break;
        }
    }

    let Some(dir) = matched else {
        return Ok(None);
    };
    let prefix = if dir.is_empty() {
        None
    } else {
        Some(format!("{dir}/"))
    };
    let selected: Vec<RemoteFile> = files
        .into_iter()
        .filter(|f| prefix.as_deref().is_none_or(|p| f.path.starts_with(p)))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    download_files(&rr, &selected, get).map(Some)
}

/// Every file under `prefix` (the whole repo when `None`), sorted by path.
///
/// One recursive `git/trees` call; if GitHub truncated the response we fall back
/// to per-directory `contents` listing, which costs one request per directory but
/// always returns a complete listing.
fn list_files_under(rr: &RepoRef, prefix: Option<&str>, get: Get) -> Result<Vec<RemoteFile>> {
    let body = get(&tree_url(&rr.owner, &rr.repo, &rr.r#ref)?)?;
    let v: Value = serde_json::from_slice(&body)?;

    let mut files = if v.get("truncated").and_then(Value::as_bool) == Some(true) {
        list_via_contents(rr, prefix, get)?
    } else {
        files_from_tree(&v, prefix)
    };
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Files of `prefix` taken from a `git/trees` response.
fn files_from_tree(tree: &Value, prefix: Option<&str>) -> Vec<RemoteFile> {
    let pref = prefix.map(|p| format!("{p}/"));
    tree.get("tree")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    let path = e.get("path")?.as_str()?;
                    if e.get("type")?.as_str()? != "blob" {
                        return None;
                    }
                    if let Some(pref) = &pref
                        && !path.starts_with(pref.as_str())
                    {
                        return None;
                    }
                    Some(RemoteFile {
                        path: path.to_string(),
                        mode: e.get("mode").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Complete listing through the `contents` API: one request per directory, which
/// sidesteps the `git/trees` truncation limit entirely.
fn list_via_contents(rr: &RepoRef, prefix: Option<&str>, get: Get) -> Result<Vec<RemoteFile>> {
    let mut files: Vec<RemoteFile> = Vec::new();
    let mut pending: Vec<String> = vec![prefix.unwrap_or("").to_string()];

    while let Some(dir) = pending.pop() {
        let body = get(&contents_url(&rr.owner, &rr.repo, &dir, &rr.r#ref)?)?;
        let v: Value = serde_json::from_slice(&body)?;
        let Some(entries) = v.as_array() else {
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unexpected response");
            return Err(SkillsError::msg(format!("GitHub API: {msg}")));
        };
        for entry in entries {
            let Some(path) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            match entry.get("type").and_then(Value::as_str) {
                Some("dir") => pending.push(path.to_string()),
                // `symlink` / `submodule` entries have no downloadable content.
                Some("file") => files.push(RemoteFile {
                    path: path.to_string(),
                    // The contents API reports no mode: the executable bit can only
                    // be restored from a tree listing.
                    mode: None,
                }),
                _ => {}
            }
        }
    }
    Ok(files)
}

/// Download `files` into a fresh temp dir at their repository-relative paths,
/// using a small pool of threads.
fn download_files(
    rr: &RepoRef,
    files: &[RemoteFile],
    get: Get,
) -> Result<(tempfile::TempDir, PathBuf)> {
    let out = tempfile::TempDir::new()?;
    let root = out.path().to_path_buf();
    if files.is_empty() {
        return Ok((out, root));
    }

    let next = AtomicUsize::new(0);
    // Only the message is kept: it must be `Send` to cross threads.
    let failure: Mutex<Option<String>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..DOWNLOAD_CONCURRENCY.min(files.len()) {
            scope.spawn(|| {
                while failure.lock().is_ok_and(|f| f.is_none()) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    if let Err(e) = download_one(rr, file, &root, get) {
                        if let Ok(mut slot) = failure.lock() {
                            *slot = Some(e.to_string());
                        }
                        break;
                    }
                }
            });
        }
    });

    if let Some(msg) = failure.into_inner().ok().flatten() {
        return Err(SkillsError::msg(msg));
    }
    Ok((out, root))
}

/// Fetch one file, resolving Git LFS pointers and restoring the executable bit.
fn download_one(rr: &RepoRef, file: &RemoteFile, root: &Path, get: Get) -> Result<()> {
    let relative = relative_path(&file.path)?;
    let mut bytes = get(&raw_url(&rr.owner, &rr.repo, &rr.r#ref, &file.path)?)?;
    // `raw.githubusercontent.com` serves the pointer, not the object it stands for.
    if is_lfs_pointer(&bytes) {
        bytes = get(&media_url(&rr.owner, &rr.repo, &rr.r#ref, &file.path)?)?;
    }

    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, &bytes)?;
    set_git_mode(&target, file.mode.as_deref());
    Ok(())
}

/// Reject absolute paths and `..` segments coming from a listing.
fn relative_path(path: &str) -> Result<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(SkillsError::msg(format!(
            "GitHub API: unsafe file path \"{path}\""
        )));
    }
    Ok(p.to_path_buf())
}

/// Whether `bytes` is a Git LFS pointer rather than the file it stands for.
fn is_lfs_pointer(bytes: &[u8]) -> bool {
    // Pointers are a three-line stub — version, oid, size — so roughly 127 bytes
    // and up depending on how many digits the size has. The upper bound only
    // guards against re-fetching a real file that happens to start with that line.
    bytes.len() <= 200 && bytes.starts_with(LFS_POINTER_PREFIX)
}

/// Give an executable file its `+x` bit back (the API listing carries the mode
/// that zip archives drop).
#[cfg(unix)]
fn set_git_mode(path: &Path, mode: Option<&str>) {
    use std::os::unix::fs::PermissionsExt;

    if mode == Some("100755")
        && let Ok(meta) = std::fs::metadata(path)
    {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

/// Windows has no executable bit to restore.
#[cfg(not(unix))]
fn set_git_mode(_path: &Path, _mode: Option<&str>) {}

/// Real HTTP GET used by default (injectable for tests).
///
/// Uses the shared proxy-aware agent, honors `GITHUB_TOKEN` to raise the API rate
/// limit and to reach private repositories, and retries transient failures.
fn http_get(url: &str) -> Result<Vec<u8>> {
    let attempt = || -> Result<Vec<u8>> {
        let mut req = crate::core::fetch::agent()
            .get(url)
            .header("User-Agent", "agents-skills");
        if let Some(token) = github_token()
            && is_github_host(url)
        {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        let mut resp = req.call()?;
        let mut reader = resp.body_mut().as_reader();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    };
    crate::core::fetch::with_retry(3, attempt)
}

/// `GITHUB_TOKEN` when set and non-empty.
fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty())
}

/// Only ever send the token to GitHub-owned hosts.
fn is_github_host(url: &str) -> bool {
    [
        "https://api.github.com/",
        "https://raw.githubusercontent.com/",
        "https://media.githubusercontent.com/",
    ]
    .iter()
    .any(|host| url.starts_with(host))
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

/// `https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={ref}`.
fn contents_url(owner: &str, repo: &str, path: &str, r#ref: &str) -> Result<String> {
    let mut u = url::Url::parse("https://api.github.com").expect("valid base URL");
    {
        let mut segs = u
            .path_segments_mut()
            .map_err(|_| SkillsError::msg("invalid GitHub API URL"))?;
        segs.push("repos").push(owner).push(repo).push("contents");
        for part in path.split('/').filter(|s| !s.is_empty()) {
            segs.push(part);
        }
        if path.is_empty() {
            // GitHub expects the trailing slash when listing the repository root.
            segs.push("");
        }
    }
    u.query_pairs_mut().append_pair("ref", r#ref);
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

/// `https://media.githubusercontent.com/media/{owner}/{repo}/{ref}/{path...}`,
/// which serves Git LFS objects.
fn media_url(owner: &str, repo: &str, r#ref: &str, path: &str) -> Result<String> {
    let mut u = url::Url::parse("https://media.githubusercontent.com").expect("valid base URL");
    u.path_segments_mut()
        .map_err(|_| SkillsError::msg("invalid media URL"))?
        .push("media")
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

    /// Tree-API response body for `(path, type, mode)` entries.
    fn tree_json(tree: &[(&str, &str, &str)], truncated: bool) -> Value {
        let entries: Vec<Value> = tree
            .iter()
            .map(|(p, t, m)| serde_json::json!({"path": p, "type": t, "mode": m}))
            .collect();
        serde_json::json!({"tree": entries, "truncated": truncated})
    }

    /// Contents-API response body listing the direct children of `dir`,
    /// derived from a flat tree listing.
    fn contents_json(tree: &[(&str, &str, &str)], dir: &str) -> Value {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut dirs: Vec<String> = Vec::new();
        let mut out: Vec<Value> = Vec::new();
        for (path, ty, _) in tree {
            let Some(rel) = path.strip_prefix(&prefix) else {
                continue;
            };
            match rel.split_once('/') {
                Some((child, _)) => {
                    let child_path = format!("{prefix}{child}");
                    if !dirs.contains(&child_path) {
                        dirs.push(child_path.clone());
                        out.push(serde_json::json!({"path": child_path, "type": "dir"}));
                    }
                }
                None => {
                    let kind = if *ty == "blob" { "file" } else { ty };
                    out.push(serde_json::json!({"path": path, "type": kind}));
                }
            }
        }
        Value::Array(out)
    }

    /// The directory a `contents` URL asks for, `""` meaning the repository root.
    fn contents_dir(url: &str) -> String {
        url.split("/contents")
            .nth(1)
            .unwrap_or_default()
            .trim_start_matches('/')
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string()
    }

    /// Build an injected `get` serving a tiny in-memory GitHub repo.
    fn fake_get<'a>(
        tree: &'a [(&'a str, &'a str, &'a str)],
        files: &'a [(&'a str, &'a str)],
        truncated: bool,
    ) -> impl Fn(&str) -> Result<Vec<u8>> + Sync + 'a {
        move |url: &str| {
            if url.contains("/git/trees/") {
                return Ok(serde_json::to_vec(&tree_json(tree, truncated)).unwrap());
            }
            if url.contains("/contents") {
                return Ok(serde_json::to_vec(&contents_json(tree, &contents_dir(url))).unwrap());
            }
            if url.contains("/repos/acme/skills") {
                return Ok(serde_json::to_vec(&serde_json::json!({
                    "default_branch": "main"
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

    const TREE: &[(&str, &str, &str)] = &[
        ("pdf/SKILL.md", "blob", "100644"),
        ("pdf/scripts/run.sh", "blob", "100755"),
        ("skills/doc/SKILL.md", "blob", "100644"),
        ("README.md", "blob", "100644"),
    ];
    const FILES: &[(&str, &str)] = &[
        ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
        ("pdf/scripts/run.sh", "#!/bin/sh\n"),
        (
            "skills/doc/SKILL.md",
            "---\nname: doc\ndescription: d\n---\nbody",
        ),
    ];

    #[test]
    fn fetch_via_api_finds_skill_dir_only() {
        let parsed = parse_source("acme/skills@pdf").unwrap();
        let get = fake_get(TREE, FILES, false);
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
        let get = fake_get(TREE, FILES, false);
        assert!(
            fetch_skill_via_api_with(&parsed, "zzz", true, &get)
                .unwrap()
                .is_none()
        );
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
            ("skills/pdf/SKILL.md", "blob", "100644"),
            ("skills/pdf/scripts/run.sh", "blob", "100755"),
            ("skills/doc/SKILL.md", "blob", "100644"),
            ("README.md", "blob", "100644"),
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
        let get = fake_get(&tree, &files, false);
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
    fn truncated_tree_falls_back_to_contents_listing() {
        // The tree API answers, but truncated: the listing must come from the
        // contents API instead of failing (or downloading the whole repo).
        let parsed = parse_source("acme/skills/skills/pdf").unwrap();
        let tree = [
            ("skills/pdf/SKILL.md", "blob", "100644"),
            ("skills/pdf/assets/logo.svg", "blob", "100644"),
            ("skills/doc/SKILL.md", "blob", "100644"),
        ];
        let files = [
            (
                "skills/pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
            ),
            ("skills/pdf/assets/logo.svg", "<svg/>"),
            (
                "skills/doc/SKILL.md",
                "---\nname: doc\ndescription: d\n---\nbody",
            ),
        ];
        let get = fake_get(&tree, &files, true);
        let (tmp, root) = fetch_subdir_via_api_with(&parsed, &get)
            .unwrap()
            .expect("truncated tree should fall back to the contents API");

        assert!(root.join("skills/pdf/SKILL.md").is_file());
        assert!(root.join("skills/pdf/assets/logo.svg").is_file());
        assert!(!root.join("skills/doc/SKILL.md").exists());
        let _ = tmp;
    }

    #[test]
    fn contents_listing_skips_symlinks_and_submodules() {
        let rr = RepoRef {
            owner: "acme".into(),
            repo: "skills".into(),
            r#ref: "main".into(),
        };
        let get = |url: &str| -> Result<Vec<u8>> {
            assert!(url.contains("/contents"), "unexpected url: {url}");
            Ok(serde_json::to_vec(&serde_json::json!([
                {"path": "pdf/SKILL.md", "type": "file"},
                {"path": "pdf/link", "type": "symlink"},
                {"path": "pdf/sub", "type": "submodule"},
            ]))
            .unwrap())
        };
        let files = list_via_contents(&rr, Some("pdf"), &get).unwrap();
        assert_eq!(
            files,
            vec![RemoteFile {
                path: "pdf/SKILL.md".into(),
                mode: None
            }]
        );
    }

    #[test]
    fn lfs_pointer_is_refetched_from_media() {
        let parsed = parse_source("acme/skills/skills/pdf").unwrap();
        let tree = [("skills/pdf/data.bin", "blob", "100644")];
        let pointer = format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 42\n",
            "0".repeat(64)
        );
        let get = move |url: &str| -> Result<Vec<u8>> {
            if url.contains("/git/trees/") {
                return Ok(serde_json::to_vec(&tree_json(&tree, false)).unwrap());
            }
            if url == "https://api.github.com/repos/acme/skills" {
                return Ok(serde_json::to_vec(&serde_json::json!({
                    "default_branch": "main"
                }))
                .unwrap());
            }
            if url.contains("media.githubusercontent.com") {
                return Ok(b"real-bytes".to_vec());
            }
            if url.ends_with("/skills/pdf/data.bin") {
                return Ok(pointer.as_bytes().to_vec());
            }
            Err(SkillsError::msg(format!("unexpected url: {url}")))
        };

        let (tmp, root) = fetch_subdir_via_api_with(&parsed, &get)
            .unwrap()
            .expect("the LFS file should be installed");
        assert_eq!(
            std::fs::read_to_string(root.join("skills/pdf/data.bin")).unwrap(),
            "real-bytes"
        );
        let _ = tmp;
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_comes_from_the_tree_mode() {
        use std::os::unix::fs::PermissionsExt;

        let parsed = parse_source("acme/skills/pdf").unwrap();
        let tree = [
            ("pdf/SKILL.md", "blob", "100644"),
            ("pdf/scripts/run.sh", "blob", "100755"),
        ];
        let files = [
            ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
            ("pdf/scripts/run.sh", "#!/bin/sh\n"),
        ];
        let get = fake_get(&tree, &files, false);
        let (tmp, root) = fetch_subdir_via_api_with(&parsed, &get)
            .unwrap()
            .expect("pdf should match");

        let mode = |p: &str| {
            std::fs::metadata(root.join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode("pdf/scripts/run.sh"), 0, "script should be executable");
        assert_eq!(mode("pdf/SKILL.md"), 0, "manifest should not be executable");
        let _ = tmp;
    }

    #[test]
    fn download_files_fetches_every_file_exactly_once() {
        let rr = RepoRef {
            owner: "acme".into(),
            repo: "skills".into(),
            r#ref: "main".into(),
        };
        let files: Vec<RemoteFile> = (0..25)
            .map(|i| RemoteFile {
                path: format!("dir/f{i}.txt"),
                mode: None,
            })
            .collect();
        let calls = AtomicUsize::new(0);
        let get = |url: &str| -> Result<Vec<u8>> {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(url.as_bytes().to_vec())
        };

        let (tmp, root) = download_files(&rr, &files, &get).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), files.len());
        for file in &files {
            assert_eq!(
                std::fs::read_to_string(root.join(&file.path)).unwrap(),
                raw_url(&rr.owner, &rr.repo, &rr.r#ref, &file.path).unwrap()
            );
        }
        let _ = tmp;
    }

    #[test]
    fn unsafe_paths_from_a_listing_are_rejected() {
        assert!(relative_path("pdf/SKILL.md").is_ok());
        assert!(relative_path("../evil").is_err());
        assert!(relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn fetch_subdir_without_files_returns_none() {
        let parsed = parse_source("acme/skills/skills/missing").unwrap();
        let tree = [("skills/pdf/SKILL.md", "blob", "100644")];
        let files = [(
            "skills/pdf/SKILL.md",
            "---\nname: pdf\ndescription: d\n---\nbody",
        )];
        let get = fake_get(&tree, &files, false);
        assert!(fetch_subdir_via_api_with(&parsed, &get).unwrap().is_none());
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
        assert_eq!(
            media_url("o", "r", "main", "a/b.bin").unwrap(),
            "https://media.githubusercontent.com/media/o/r/main/a/b.bin"
        );
    }

    #[test]
    fn contents_url_targets_the_requested_directory() {
        assert_eq!(
            contents_url("o", "r", "skills/pdf", "main").unwrap(),
            "https://api.github.com/repos/o/r/contents/skills/pdf?ref=main"
        );
        // The repository root keeps GitHub's trailing slash.
        assert_eq!(
            contents_url("o", "r", "", "main").unwrap(),
            "https://api.github.com/repos/o/r/contents/?ref=main"
        );
        // A branch name with a slash survives as a query value.
        assert_eq!(
            contents_url("o", "r", "pdf", "feat/x").unwrap(),
            "https://api.github.com/repos/o/r/contents/pdf?ref=feat%2Fx"
        );
    }

    #[test]
    fn token_is_only_sent_to_github_hosts() {
        assert!(is_github_host("https://api.github.com/repos/o/r"));
        assert!(is_github_host(
            "https://raw.githubusercontent.com/o/r/main/x"
        ));
        assert!(is_github_host(
            "https://media.githubusercontent.com/media/o/r/main/x"
        ));
        assert!(!is_github_host("https://evil.example/api.github.com/"));
        assert!(!is_github_host(
            "https://codeload.github.com/o/r/tar.gz/main"
        ));
    }
}
