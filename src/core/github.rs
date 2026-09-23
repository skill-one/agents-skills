//! GitHub API: fetch exactly one named skill from a repository.
//!
//! Listing is one recursive `git/trees` call. When GitHub truncates that tree
//! (large repos) we switch to per-directory `contents` calls — the workaround
//! the official docs recommend — instead of failing. A skill's name is its
//! **directory name**: the requested name matches a directory that directly
//! contains `SKILL.md` (case-insensitive, shallowest match wins), and a
//! `SKILL.md` at the repository root is selected by the repository name.
//! Nothing is downloaded until the directory is located; the matched
//! directory's files are then fetched concurrently from
//! `raw.githubusercontent.com`, Git LFS pointers are resolved through
//! `media.githubusercontent.com`, and the git mode from the tree listing
//! restores the executable bit that zip archives lose.
//!
//! The HTTP client is injected as a `get` closure so the whole selection logic is
//! unit-testable offline; production uses a plain ureq GET.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::core::discover::{Skill, read_description, read_skill};
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
    /// Resolve the ref (explicitly requested, or the repository's default branch).
    fn resolve(owner: &str, repo: &str, reference: Option<&str>, get: Get) -> Result<Self> {
        let r#ref = match reference {
            Some(r) => r.to_string(),
            None => {
                let body = get(&repo_url(owner, repo))?;
                let v: Value = serde_json::from_slice(&body)?;
                v.get("default_branch")
                    .and_then(|b| b.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| SkillsError::msg("GitHub API: missing default_branch"))?
            }
        };
        Ok(RepoRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            r#ref,
        })
    }
}

/// Fetch the named skill's directory from GitHub into a fresh temp dir.
///
/// `Ok((temp, skill))` holds the matched skill; the caller keeps `temp` alive
/// until the skill has been installed. `Ok` never carries "not found" — an
/// unknown skill name is an error naming the skill and the repository.
pub fn fetch_skill(
    owner: &str,
    repo: &str,
    skill_name: &str,
    reference: Option<&str>,
) -> Result<(tempfile::TempDir, Skill)> {
    let slug = format!("{owner}/{repo}");
    match fetch_skill_with(owner, repo, skill_name, reference, &http_get) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(SkillsError::msg(not_found_message(skill_name, &slug))),
        Err(e) => Err(decorate_api_error(e, &slug)),
    }
}

/// The error for a skill name the repository does not contain.
fn not_found_message(skill_name: &str, slug: &str) -> String {
    format!(
        "No skill directory named \"{skill_name}\" found in {slug}. \
         The name after @ matches a directory containing SKILL.md \
         (case-insensitive); a SKILL.md at the repository root is selected \
         with the repository name."
    )
}

/// Prefix transport/API errors with the repository they came from and append the
/// rate-limit hint when the failure looks like one.
fn decorate_api_error(e: SkillsError, slug: &str) -> SkillsError {
    let hint = match &e {
        SkillsError::Http(he) if matches!(he.as_ref(), ureq::Error::StatusCode(403 | 429)) => {
            " Set GITHUB_TOKEN to raise the API rate limit from 60 to 5000 requests/hour."
        }
        _ => "",
    };
    SkillsError::msg(format!("GitHub API request failed for {slug}: {e}.{hint}"))
}

/// The skill directory a file belongs to when the file is a `SKILL.md`:
/// `""` for a root-level manifest, otherwise the path without its tail.
fn skill_dir_of(path: &str) -> Option<&str> {
    match path {
        "SKILL.md" => Some(""),
        p => p.strip_suffix("/SKILL.md"),
    }
}

/// The last path segment of a repository-relative dir.
fn dir_basename(dir: &str) -> &str {
    dir.rsplit('/').next().unwrap_or(dir)
}

/// Injectable core of [`fetch_skill`]: `Ok(None)` means the API worked but no
/// skill directory matched the name.
fn fetch_skill_with(
    owner: &str,
    repo: &str,
    skill_name: &str,
    reference: Option<&str>,
    get: Get,
) -> Result<Option<(tempfile::TempDir, Skill)>> {
    let rr = RepoRef::resolve(owner, repo, reference, get)?;
    let files = list_files(&rr, get)?;

    // Locate the skill directory directly from the listing — no downloads:
    // a directory containing SKILL.md whose name matches (case-insensitive).
    // A root-level manifest has no directory name, so it takes the repo name.
    // Shallowest match wins, ties broken by path for determinism.
    let mut matched: Option<(usize, String)> = None;
    for f in &files {
        let Some(dir) = skill_dir_of(&f.path) else {
            continue;
        };
        let depth = if dir.is_empty() {
            0
        } else {
            dir.matches('/').count() + 1
        };
        let name = if dir.is_empty() {
            rr.repo.as_str()
        } else {
            dir_basename(dir)
        };
        if name.eq_ignore_ascii_case(skill_name) {
            let better = match &matched {
                None => true,
                Some((d, p)) => (depth, dir) < (*d, p.as_str()),
            };
            if better {
                matched = Some((depth, dir.to_string()));
            }
        }
    }

    let Some((_, dir)) = matched else {
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
    let (temp, root) = download_files(&rr, &selected, get)?;

    // Build the skill from the downloaded directory. Its name is the directory
    // name; the manifest contributes only the description. Explicit selection
    // also makes internal skills visible. The root case has no directory name,
    // so the skill is named after the repository.
    let skill = if dir.is_empty() {
        Skill {
            name: rr.repo.clone(),
            description: read_description(&root.join("SKILL.md")),
            dir: root,
        }
    } else {
        match read_skill(&root.join(&dir), true) {
            Some(s) => s,
            None => return Ok(None),
        }
    };
    Ok(Some((temp, skill)))
}

/// Every file in the repository, sorted by path.
///
/// One recursive `git/trees` call; if GitHub truncated the response we fall back
/// to per-directory `contents` listing, which costs one request per directory but
/// always returns a complete listing.
fn list_files(rr: &RepoRef, get: Get) -> Result<Vec<RemoteFile>> {
    let body = get(&tree_url(&rr.owner, &rr.repo, &rr.r#ref)?)?;
    let v: Value = serde_json::from_slice(&body)?;

    let mut files = if v.get("truncated").and_then(Value::as_bool) == Some(true) {
        list_via_contents(rr, get)?
    } else {
        files_from_tree(&v)
    };
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Files taken from a `git/trees` response (blobs only).
fn files_from_tree(tree: &Value) -> Vec<RemoteFile> {
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
fn list_via_contents(rr: &RepoRef, get: Get) -> Result<Vec<RemoteFile>> {
    let mut files: Vec<RemoteFile> = Vec::new();
    let mut pending: Vec<String> = vec![String::new()];

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
                    // The contents API reports no mode: the executable bit can
                    // only be restored from a tree listing.
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

// ============================================================================
// HTTP transport (proxy-aware, token-aware, retried).
// ============================================================================

/// Shared HTTP agent: honors `HTTP(S)_PROXY` / `ALL_PROXY` / `NO_PROXY` env vars
/// (ureq reads them via `Proxy::try_from_env`) so proxied networks can reach GitHub.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let mut builder = ureq::Agent::config_builder();
        if let Some(proxy) = ureq::Proxy::try_from_env() {
            builder = builder.proxy(Some(proxy));
        }
        ureq::Agent::new_with_config(builder.build())
    })
}

/// Run `f` up to `attempts` times with exponential backoff between failures
/// (150ms, 300ms, ...). Used around network calls to survive transient drops.
fn with_retry<T>(attempts: usize, mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last: Option<SkillsError> = None;
    for i in 0..attempts {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if i + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(150 * (1 << i)));
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| SkillsError::msg("retry exhausted")))
}

/// Real HTTP GET used by default (injectable for tests).
///
/// Uses the shared proxy-aware agent, honors `GITHUB_TOKEN` to raise the API rate
/// limit and to reach private repositories, and retries transient failures.
fn http_get(url: &str) -> Result<Vec<u8>> {
    let attempt = || -> Result<Vec<u8>> {
        let mut req = agent().get(url).header("User-Agent", "agents-skills");
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
    with_retry(3, attempt)
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
        for (path, ty, _m) in tree {
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
                return Ok(
                    serde_json::to_vec(&serde_json::json!({"default_branch": "main"})).unwrap(),
                );
            }
            for (p, content) in files {
                // raw URLs percent-encode path segments (e.g. spaces).
                let suffix = format!("/{}", p.replace(' ', "%20"));
                if url.ends_with(&suffix) {
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
    fn fetch_finds_the_named_skill_dir_only() {
        let get = fake_get(TREE, FILES, false);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");

        assert_eq!(skill.name, "pdf");
        let root = tmp.path();
        assert!(root.join("pdf/SKILL.md").is_file());
        assert!(root.join("pdf/scripts/run.sh").is_file());
        // Only the matched skill dir is fetched.
        assert!(!root.join("skills/doc/SKILL.md").exists());
        assert!(!root.join("README.md").exists());
    }

    #[test]
    fn fetch_matches_on_directory_name_ignoring_frontmatter_name() {
        // The skill name is the directory name; the frontmatter `name` is
        // ignored. Matching is case-insensitive.
        let tree = [("skills/PDF Master/SKILL.md", "blob", "100644")];
        let files = [(
            "skills/PDF Master/SKILL.md",
            "---\nname: pdf-skill\ndescription: d\n---\nbody",
        )];
        let get = fake_get(&tree, &files, false);
        let (_tmp, skill) = fetch_skill_with("acme", "skills", "pdf master", None, &get)
            .unwrap()
            .expect("directory name should match");
        assert_eq!(skill.name, "PDF Master");
        assert_eq!(skill.description, "d");
    }

    #[test]
    fn fetch_does_not_match_on_frontmatter_name() {
        // The directory is `acrobat`; a frontmatter `name: pdf` must not make
        // `@pdf` match anymore.
        let tree = [("skills/acrobat/SKILL.md", "blob", "100644")];
        let files = [(
            "skills/acrobat/SKILL.md",
            "---\nname: pdf\ndescription: d\n---\nbody",
        )];
        let get = fake_get(&tree, &files, false);
        assert!(
            fetch_skill_with("acme", "skills", "pdf", None, &get)
                .unwrap()
                .is_none()
        );
        // The directory name still selects it.
        let (_tmp, skill) = fetch_skill_with("acme", "skills", "acrobat", None, &get)
            .unwrap()
            .expect("directory name should match");
        assert_eq!(skill.name, "acrobat");
    }

    #[test]
    fn fetch_root_skill_md_is_selected_by_repo_name() {
        // A SKILL.md at the repository root is a skill named after the repo;
        // selecting it downloads the whole repository.
        let tree = [
            ("SKILL.md", "blob", "100644"),
            ("README.md", "blob", "100644"),
        ];
        let files = [
            (
                "SKILL.md",
                "---\nname: whatever\ndescription: root skill\n---\nbody",
            ),
            ("README.md", "# repo"),
        ];
        let get = fake_get(&tree, &files, false);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "skills", None, &get)
            .unwrap()
            .expect("root manifest is selected with the repo name");
        assert_eq!(skill.name, "skills");
        assert_eq!(skill.description, "root skill");
        assert!(tmp.path().join("SKILL.md").is_file());
        assert!(tmp.path().join("README.md").is_file());
    }

    #[test]
    fn fetch_shallowest_directory_wins() {
        // Two dirs with the same basename: the shallower one is selected.
        let tree = [
            ("pdf/SKILL.md", "blob", "100644"),
            ("vendor/pdf/SKILL.md", "blob", "100644"),
        ];
        let files = [
            (
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: shallow\n---\nbody",
            ),
            (
                "vendor/pdf/SKILL.md",
                "---\nname: pdf\ndescription: deep\n---\nbody",
            ),
        ];
        let get = fake_get(&tree, &files, false);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");
        assert_eq!(skill.description, "shallow");
        assert!(tmp.path().join("pdf/SKILL.md").is_file());
        assert!(!tmp.path().join("vendor/pdf/SKILL.md").exists());
    }

    #[test]
    fn fetch_no_match_returns_none() {
        let get = fake_get(TREE, FILES, false);
        assert!(
            fetch_skill_with("acme", "skills", "zzz", None, &get)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fetch_public_api_error_propagates() {
        let get = |url: &str| -> Result<Vec<u8>> {
            Err(SkillsError::msg(format!("network down: {url}")))
        };
        assert!(fetch_skill_with("acme", "skills", "pdf", None, &get).is_err());
    }

    #[test]
    fn fetch_public_not_found_is_a_clean_message() {
        let msg = not_found_message("zzz", "acme/skills");
        assert!(msg.contains("No skill directory named \"zzz\""), "{msg}");
        assert!(msg.contains("acme/skills"), "{msg}");
        assert!(!msg.contains("request failed"), "{msg}");
    }

    #[test]
    fn truncated_tree_falls_back_to_contents_listing() {
        // The tree API answers, but truncated: the listing comes from the
        // contents API instead of failing.
        let tree = [
            ("pdf/SKILL.md", "blob", "100644"),
            ("pdf/assets/logo.svg", "blob", "100644"),
            ("doc/SKILL.md", "blob", "100644"),
        ];
        let files = [
            ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
            ("pdf/assets/logo.svg", "<svg/>"),
            ("doc/SKILL.md", "---\nname: doc\ndescription: d\n---\nbody"),
        ];
        let get = fake_get(&tree, &files, true);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("truncated tree should fall back to the contents API");
        assert_eq!(skill.name, "pdf");
        let root = tmp.path();
        assert!(root.join("pdf/SKILL.md").is_file());
        assert!(root.join("pdf/assets/logo.svg").is_file());
        assert!(!root.join("doc/SKILL.md").exists());
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
        let files = list_via_contents(&rr, &get).unwrap();
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
        let pointer = format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 42\n",
            "0".repeat(64)
        );
        // The manifest must exist for `pdf` to be a candidate.
        let tree = [
            ("pdf/SKILL.md", "blob", "100644"),
            ("pdf/data.bin", "blob", "100644"),
        ];
        let get = move |url: &str| -> Result<Vec<u8>> {
            if url.contains("/git/trees/") {
                return Ok(serde_json::to_vec(&tree_json(&tree, false)).unwrap());
            }
            if url == "https://api.github.com/repos/acme/skills" {
                return Ok(
                    serde_json::to_vec(&serde_json::json!({"default_branch": "main"})).unwrap(),
                );
            }
            if url.contains("media.githubusercontent.com") {
                return Ok(b"real-bytes".to_vec());
            }
            if url.ends_with("/pdf/SKILL.md") {
                return Ok(b"---\nname: pdf\ndescription: d\n---\nbody".to_vec());
            }
            if url.ends_with("/pdf/data.bin") {
                return Ok(pointer.as_bytes().to_vec());
            }
            Err(SkillsError::msg(format!("unexpected url: {url}")))
        };

        let (tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("the LFS file should be installed");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("pdf/data.bin")).unwrap(),
            "real-bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_comes_from_the_tree_mode() {
        use std::os::unix::fs::PermissionsExt;

        let tree = [
            ("pdf/SKILL.md", "blob", "100644"),
            ("pdf/scripts/run.sh", "blob", "100755"),
        ];
        let files = [
            ("pdf/SKILL.md", "---\nname: pdf\ndescription: d\n---\nbody"),
            ("pdf/scripts/run.sh", "#!/bin/sh\n"),
        ];
        let get = fake_get(&tree, &files, false);
        let (tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");

        let mode = |p: &str| {
            std::fs::metadata(tmp.path().join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode("pdf/scripts/run.sh"), 0, "script should be executable");
        assert_eq!(mode("pdf/SKILL.md"), 0, "manifest should not be executable");
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
    }

    #[test]
    fn rate_limit_hint_decorates_403_and_429() {
        let limited = SkillsError::Http(Box::new(ureq::Error::StatusCode(403)));
        assert!(
            decorate_api_error(limited, "o/r")
                .to_string()
                .contains("GITHUB_TOKEN")
        );
        let limited = SkillsError::Http(Box::new(ureq::Error::StatusCode(429)));
        assert!(
            decorate_api_error(limited, "o/r")
                .to_string()
                .contains("GITHUB_TOKEN")
        );
        // A 5xx or a plain message is prefixed but gets no rate-limit advice.
        let unavailable = SkillsError::Http(Box::new(ureq::Error::StatusCode(503)));
        let msg = decorate_api_error(unavailable, "o/r").to_string();
        assert!(msg.contains("request failed"));
        assert!(!msg.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn with_retry_succeeds_after_failures() {
        let mut calls = 0;
        let r = with_retry(3, || -> Result<i32> {
            calls += 1;
            if calls < 3 {
                Err(SkillsError::msg("boom"))
            } else {
                Ok(42)
            }
        });
        assert_eq!(r.unwrap(), 42);
        assert_eq!(calls, 3);
    }

    #[test]
    fn with_retry_exhausts_after_attempts() {
        let mut calls = 0;
        let r = with_retry(2, || -> Result<i32> {
            calls += 1;
            Err(SkillsError::msg("boom"))
        });
        assert!(r.is_err());
        assert_eq!(calls, 2);
    }
}
