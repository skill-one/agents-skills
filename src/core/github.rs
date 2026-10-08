//! Fetch exactly one named skill from a GitHub repository with a single request.
//!
//! The whole repository tarball is downloaded from `codeload.github.com` — the
//! GitHub REST API is never touched, so the anonymous 60-requests-per-hour API
//! rate limit does not apply. The archive is unpacked into a temp dir and the
//! skill is matched locally: every directory that directly contains `SKILL.md`
//! is a candidate, and the requested slug is matched against the slugified
//! `name` of each manifest's frontmatter (first match in shallowest-then-path
//! order wins). A `SKILL.md` at the repository root is an ordinary candidate —
//! when it matches, the whole repository is the skill. The installed directory
//! keeps the matched directory's name in the source repository, verbatim.
//! Nothing but the single tarball request is needed; public repositories only,
//! and Git LFS files install as their pointer stubs.
//!
//! The HTTP client is injected as a `get` closure so the whole selection logic is
//! unit-testable offline; production uses a plain ureq GET.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

use crate::core::discover::{Skill, read_skill};
use crate::core::install::slugify;
use crate::error::{Result, SkillsError};

/// Injected HTTP GET: returns a streaming reader of the response body of `url`.
type Get<'a> = &'a (dyn Fn(&str) -> Result<Box<dyn Read>> + Sync);

/// Fetch the skill selected by `owner/repo/{skill_slug}` into a fresh temp dir.
///
/// `Ok((temp, skill))` holds the matched skill; the caller keeps `temp` alive
/// until the skill has been installed. `Ok` never carries "not found" — an
/// unknown slug is an error naming the slug and the repository.
pub fn fetch_skill(
    owner: &str,
    repo: &str,
    skill_slug: &str,
    reference: Option<&str>,
) -> Result<(tempfile::TempDir, Skill)> {
    let slug = format!("{owner}/{repo}");
    match fetch_skill_with(owner, repo, skill_slug, reference, &http_get_stream) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(SkillsError::msg(not_found_message(skill_slug, &slug))),
        Err(e) => Err(decorate_download_error(e, &slug)),
    }
}

/// The error for a slug no manifest in the repository matches.
fn not_found_message(skill_slug: &str, slug: &str) -> String {
    format!(
        "No skill with slug \"{skill_slug}\" found in {slug}. The id's last \
         segment matches the slugified `name` of a SKILL.md frontmatter \
         (lowercase, spaces as dashes, `/` dropped); the first matching \
         manifest in the repository tree is installed."
    )
}

/// Prefix transport errors with the repository they came from; a 404 means the
/// repository or the requested ref does not exist (public repositories only).
fn decorate_download_error(e: SkillsError, slug: &str) -> SkillsError {
    let prefix = if is_http_status(&e, 404) {
        format!(
            "GitHub repository or ref not found: {slug}. \
             Check the repository name and --ref (a branch, tag, or full \
             40-character commit SHA)."
        )
    } else {
        format!("GitHub download failed for {slug}.")
    };
    SkillsError::msg(format!("{prefix} {e}"))
}

/// Clear the contents of a directory (used before retrying an unpack attempt).
fn clear_dir(dir: &Path) -> Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

/// Download the repository tarball (one request per ref form, at most two) and
/// unpack it into `root`, returning the unpacked repository root directory.
fn download_repo(
    owner: &str,
    repo: &str,
    reference: Option<&str>,
    root: &Path,
    get: Get,
) -> Result<PathBuf> {
    let mut last: Option<SkillsError> = None;
    for form in ref_forms(reference) {
        let url = archive_url(owner, repo, &form);
        let mut attempt = || -> Result<PathBuf> {
            clear_dir(root)?;
            let stream = get(&url)?;
            unpack_archive(stream, root)?;
            repo_root_of(root, repo)
        };
        match with_retry(3, &mut attempt) {
            Ok(p) => return Ok(p),
            Err(e) => {
                // Only a 404 falls through to the next ref form (branch → tag);
                // anything else is a real failure.
                if !is_http_status(&e, 404) {
                    return Err(e);
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| SkillsError::msg("no ref form tried")))
}

/// The archive ref forms to try, in order.
///
/// Without an explicit ref the default branch is addressed as `HEAD`. A full
/// 40-character commit SHA is used as-is. Anything else is a branch first,
/// then a tag — abbreviated SHAs are not supported.
fn ref_forms(reference: Option<&str>) -> Vec<String> {
    match reference {
        None => vec!["HEAD".to_string()],
        Some(r) if is_full_sha(r) => vec![r.to_string()],
        Some(r) => vec![format!("refs/heads/{r}"), format!("refs/tags/{r}")],
    }
}

/// Whether `r` is a full 40-character hex commit SHA.
fn is_full_sha(r: &str) -> bool {
    r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `https://codeload.github.com/{owner}/{repo}/tar.gz/{form}`.
fn archive_url(owner: &str, repo: &str, form: &str) -> String {
    format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{form}")
}

/// Whether `e` is the HTTP error for `status`.
fn is_http_status(e: &SkillsError, status: u16) -> bool {
    matches!(
        e,
        SkillsError::Http(h)
            if matches!(h.as_ref(), ureq::Error::StatusCode(c) if *c == status)
    )
}

/// Unpack a gzipped tar archive from a streaming reader into `root`.
///
/// Only regular files are unpacked: directories are created on demand, and
/// symlinks, hardlinks, and metadata headers are never part of a skill.
/// `unpack_in` refuses entries whose path would escape `root`, so a crafted
/// archive cannot write outside the temp dir.
fn unpack_archive<R: Read>(reader: R, root: &Path) -> Result<()> {
    let mut archive = Archive::new(GzDecoder::new(reader));
    archive.set_preserve_permissions(true);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type() != EntryType::Regular {
            continue;
        }
        entry.unpack_in(root)?;
    }
    Ok(())
}

/// Consolidate the unpacked repository root directory, named after the repo.
///
/// GitHub tarballs hold one top-level `<repo>-<ref>` directory; it is renamed
/// to the plain repository name, so the skill directory's basename — the
/// install slot a root-manifest skill gets — is meaningful. Any other layout
/// (no or several top-level entries) is consolidated under `repo/` the same
/// way. Rename failures are propagated: silently continuing would leave the
/// returned root missing and surface later as a misleading "no skill found".
fn repo_root_of(root: &Path, repo: &str) -> Result<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(root.to_path_buf());
    };
    let paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    let target = root.join(repo);
    if paths.len() == 1 && paths[0].is_dir() {
        if paths[0] != target {
            fs::rename(&paths[0], &target)
                .map_err(|e| SkillsError::msg(format!("rename {}: {e}", paths[0].display())))?;
        }
        return Ok(target);
    }
    if paths.is_empty() {
        return Ok(root.to_path_buf());
    }
    if fs::create_dir(&target).is_ok() || target.is_dir() {
        for path in &paths {
            let Some(name) = path.file_name() else {
                continue;
            };
            fs::rename(path, target.join(name))
                .map_err(|e| SkillsError::msg(format!("rename {}: {e}", path.display())))?;
        }
        return Ok(target);
    }
    Err(SkillsError::msg(format!(
        "create {}: cannot create the repository root",
        target.display()
    )))
}

/// The skill matching [`fetch_skill`] promises, run on the unpacked tree.
///
/// Every directory that directly contains a `SKILL.md` is a candidate: each
/// manifest's slugified frontmatter `name` is matched against the requested
/// slug, and the first match in (depth, path) order wins. A manifest without
/// a frontmatter declaring a non-empty `name` is not a skill and never
/// matches. The root manifest is an ordinary candidate: when it matches, the
/// whole repository is the skill.
fn select_skill(repo_root: &Path, skill_slug: &str) -> Result<Option<Skill>> {
    let requested = slugify(skill_slug);
    let mut candidates: Vec<(usize, String)> = Vec::new();
    collect_manifest_dirs(repo_root, "", &mut candidates)?;
    candidates.sort();

    for (_, dir) in candidates {
        let path = repo_root.join(&dir);
        // Explicit selection also makes internal skills visible.
        let Some(skill) = read_skill(&path) else {
            continue;
        };
        if slugify(&skill.name) != requested {
            continue;
        }
        return Ok(Some(skill));
    }
    Ok(None)
}

/// Every directory under `dir` (relative path `rel`) that directly contains a
/// `SKILL.md`. Hidden entries (names starting with `.`) are scanned like any
/// other directory: a skill may legitimately live under a dot-directory.
fn collect_manifest_dirs(dir: &Path, rel: &str, out: &mut Vec<(usize, String)>) -> Result<()> {
    if dir.join("SKILL.md").is_file() {
        let depth = if rel.is_empty() {
            0
        } else {
            rel.matches('/').count() + 1
        };
        out.push((depth, rel.to_string()));
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        collect_manifest_dirs(&entry.path(), &child_rel, out)?;
    }
    Ok(())
}

/// Injectable core of [`fetch_skill`]: `Ok(None)` means the download worked but
/// no manifest's slugified name matched the requested slug.
fn fetch_skill_with(
    owner: &str,
    repo: &str,
    skill_slug: &str,
    reference: Option<&str>,
    get: Get,
) -> Result<Option<(tempfile::TempDir, Skill)>> {
    let temp = tempfile::TempDir::new()?;
    let repo_root = download_repo(owner, repo, reference, temp.path(), get)?;
    match select_skill(&repo_root, skill_slug)? {
        Some(skill) => Ok(Some((temp, skill))),
        None => Ok(None),
    }
}

// ============================================================================
// HTTP transport (proxy-aware, retried).
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
        builder = builder
            .timeout_connect(Some(std::time::Duration::from_secs(15)))
            .timeout_global(Some(std::time::Duration::from_secs(60)));
        ureq::Agent::new_with_config(builder.build())
    })
}

/// Run `f` up to `attempts` times with exponential backoff between failures
/// (150ms, 300ms, ...). Deterministic client errors (404 and friends) are
/// returned immediately — retrying cannot fix them.
fn with_retry<T>(attempts: usize, mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last: Option<SkillsError> = None;
    for i in 0..attempts {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !retriable(&e) {
                    return Err(e);
                }
                last = Some(e);
                if i + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(150 * (1 << i)));
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| SkillsError::msg("retry exhausted")))
}

/// Whether retrying might help: anything but a deterministic 4xx (429 is 4xx
/// too, but rate limiting is transient).
fn retriable(e: &SkillsError) -> bool {
    !matches!(
        e,
        SkillsError::Http(h)
            if matches!(h.as_ref(), ureq::Error::StatusCode(c)
                if (400..500).contains(c) && *c != 429)
    )
}

/// Real HTTP GET used by default (injectable for tests).
///
/// Uses the shared proxy-aware agent and retries transient failures. No
/// authentication: only public repositories are supported. The body is capped
/// at [`MAX_TARBALL_BYTES`] so a runaway response cannot exhaust memory.
fn http_get_stream(url: &str) -> Result<Box<dyn Read>> {
    let resp = agent()
        .get(url)
        .header("User-Agent", "agents-skills")
        .call()?;
    let reader = resp.into_body().into_reader().take(MAX_TARBALL_BYTES);
    Ok(Box::new(reader))
}

/// Largest accepted response body: 512 MiB, far above any real skill repo.
const MAX_TARBALL_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Build a GitHub-style tarball: one top dir `prefix/`, then regular file
    /// entries `(repo-relative path, content, mode)` and symlink entries
    /// `(path, target)` under it.
    fn tarball(prefix: &str, files: &[(&str, &str, u32)], links: &[(&str, &str)]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, content, mode) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_entry_type(EntryType::Regular);
            header.set_mtime(0);
            builder
                .append_data(&mut header, format!("{prefix}/{path}"), content.as_bytes())
                .unwrap();
        }
        for (path, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(EntryType::Symlink);
            header.set_mtime(0);
            builder
                .append_link(&mut header, format!("{prefix}/{path}"), target)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// Requested URLs, recorded in order.
    type UrlLog = std::sync::Arc<Mutex<Vec<String>>>;

    /// A fake HTTP GET, injectable into `fetch_skill_with`.
    type FakeGet = Box<dyn Fn(&str) -> Result<Box<dyn Read>> + Send + Sync>;

    /// An injected `get` serving `body` for every URL ending in one of `forms`,
    /// and a 404 for everything else. Records the requested URLs in order.
    fn fake_get(forms: &[&str], body: Vec<u8>) -> (FakeGet, UrlLog) {
        let forms: Vec<String> = forms.iter().map(|f| (*f).to_string()).collect();
        let requested = std::sync::Arc::new(Mutex::new(Vec::new()));
        let urls = requested.clone();
        let get = move |url: &str| -> Result<Box<dyn Read>> {
            requested.lock().unwrap().push(url.to_string());
            if forms.iter().any(|f| url.ends_with(f.as_str())) {
                Ok(Box::new(std::io::Cursor::new(body.clone())))
            } else {
                Err(SkillsError::Http(Box::new(ureq::Error::StatusCode(404))))
            }
        };
        (Box::new(get), urls)
    }

    const FORM_HEAD: &str = "tar.gz/HEAD";

    #[test]
    fn fetch_finds_the_named_skill_dir_only() {
        let body = tarball(
            "skills-main",
            &[
                (
                    "pdf/SKILL.md",
                    "---\nname: pdf\ndescription: d\n---\nbody",
                    0o644,
                ),
                ("pdf/scripts/run.sh", "#!/bin/sh\n", 0o755),
                (
                    "skills/doc/SKILL.md",
                    "---\nname: doc\ndescription: d\n---\nbody",
                    0o644,
                ),
                ("README.md", "# repo", 0o644),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");

        assert_eq!(skill.name, "pdf");
        let root = tmp.path().join("skills");
        assert!(root.join("pdf/SKILL.md").is_file());
        assert!(root.join("pdf/scripts/run.sh").is_file());
        // The whole repo is unpacked; only the *selection* is narrowed.
        assert!(root.join("skills/doc/SKILL.md").is_file());
        assert!(root.join("README.md").is_file());
        assert_eq!(skill.dir, root.join("pdf"));
    }

    #[test]
    fn fetch_matches_on_the_slugified_frontmatter_name() {
        // The requested slug is compared against the slugified `name` field
        // parsed from SKILL.md; the directory name is irrelevant to matching.
        // Matching is case-insensitive (slugs are lowercase on both sides).
        let body = tarball(
            "skills-main",
            &[(
                "skills/PDF Master/SKILL.md",
                "---\nname: PDF Skill\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (_tmp, skill) = fetch_skill_with("acme", "skills", "pdf-skill", None, &get)
            .unwrap()
            .expect("slugified frontmatter name should match");
        assert_eq!(skill.name, "PDF Skill");
        assert_eq!(skill.description, "d");
        // The install slot is the directory's basename in the repo, verbatim.
        assert_eq!(skill.dir.file_name().unwrap(), "PDF Master");
    }

    #[test]
    fn fetch_does_not_match_on_directory_name() {
        // The directory is `acrobat`; its frontmatter declares `name: pdf`.
        // Only the slugified frontmatter name selects the skill.
        let body = tarball(
            "skills-main",
            &[(
                "skills/acrobat/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        assert!(
            fetch_skill_with("acme", "skills", "acrobat", None, &get)
                .unwrap()
                .is_none()
        );
        let (_tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("frontmatter name should match");
        assert_eq!(skill.name, "pdf");
        assert_eq!(skill.dir.file_name().unwrap(), "acrobat");
    }

    #[test]
    fn fetch_slug_keeps_punctuation_verbatim() {
        // Slugify only lowercases, turns spaces into dashes, and drops `/`:
        // everything else survives, so `c++ & rust.net` matches its own slug.
        let body = tarball(
            "skills-main",
            &[(
                "lang/SKILL.md",
                "---\nname: C++ & Rust.net\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (_tmp, skill) = fetch_skill_with("acme", "skills", "c++-&-rust.net", None, &get)
            .unwrap()
            .expect("punctuation-preserving slug should match");
        assert_eq!(skill.name, "C++ & Rust.net");
        assert_eq!(skill.dir.file_name().unwrap(), "lang");
    }

    #[test]
    fn fetch_root_skill_md_is_selected_by_its_name() {
        // A SKILL.md at the repository root is an ordinary candidate: when it
        // matches, the whole repository is the skill.
        let body = tarball(
            "skills-main",
            &[
                (
                    "SKILL.md",
                    "---\nname: whatever\ndescription: root skill\n---\nbody",
                    0o644,
                ),
                ("README.md", "# repo", 0o644),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "whatever", None, &get)
            .unwrap()
            .expect("root manifest is selected with its frontmatter name");
        assert_eq!(skill.name, "whatever");
        assert_eq!(skill.description, "root skill");
        let root = tmp.path().join("skills");
        assert_eq!(skill.dir, root);
        assert!(root.join("SKILL.md").is_file());
        assert!(root.join("README.md").is_file());
    }

    #[test]
    fn fetch_shallowest_directory_wins() {
        // Two dirs with the same basename: the shallower one is selected.
        let body = tarball(
            "skills-main",
            &[
                (
                    "pdf/SKILL.md",
                    "---\nname: pdf\ndescription: shallow\n---\nbody",
                    0o644,
                ),
                (
                    "vendor/pdf/SKILL.md",
                    "---\nname: pdf\ndescription: deep\n---\nbody",
                    0o644,
                ),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");
        assert_eq!(skill.description, "shallow");
        let root = tmp.path().join("skills");
        assert!(root.join("pdf/SKILL.md").is_file());
        assert_eq!(skill.dir, root.join("pdf"));
    }

    #[test]
    fn fetch_unmatched_name_returns_none_even_with_root_manifest() {
        // No SKILL.md in the tree declares the requested name: the root
        // manifest no longer silently installs the whole repository — the
        // selector matches frontmatter names only.
        let body = tarball(
            "skills-main",
            &[
                (
                    "SKILL.md",
                    "---\nname: whatever\ndescription: root skill\n---\nbody",
                    0o644,
                ),
                (
                    "pdf/SKILL.md",
                    "---\nname: pdf\ndescription: d\n---\nbody",
                    0o644,
                ),
                ("README.md", "# repo", 0o644),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        assert!(
            fetch_skill_with("acme", "skills", "typo-name", None, &get)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fetch_first_match_in_shallowest_then_path_order_wins() {
        // Several manifests declare the same name: the first candidate in
        // (depth, path) order is installed. `a` precedes `b` at equal depth.
        let body = tarball(
            "skills-main",
            &[
                (
                    "b/SKILL.md",
                    "---\nname: pdf\ndescription: b\n---\nbody",
                    0o644,
                ),
                (
                    "a/SKILL.md",
                    "---\nname: pdf\ndescription: a\n---\nbody",
                    0o644,
                ),
                (
                    "deep/nested/SKILL.md",
                    "---\nname: pdf\ndescription: deep\n---\nbody",
                    0o644,
                ),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");
        assert_eq!(skill.description, "a");
        assert_eq!(skill.dir, tmp.path().join("skills").join("a"));
    }

    #[test]
    fn fetch_no_match_returns_none() {
        let body = tarball(
            "skills-main",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        assert!(
            fetch_skill_with("acme", "skills", "zzz", None, &get)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fetch_public_api_error_propagates() {
        let get = |url: &str| -> Result<Box<dyn Read>> {
            Err(SkillsError::msg(format!("network down: {url}")))
        };
        assert!(fetch_skill_with("acme", "skills", "pdf", None, &get).is_err());
    }

    #[test]
    fn fetch_public_not_found_is_a_clean_message() {
        let msg = not_found_message("zzz", "acme/skills");
        assert!(msg.contains("No skill with slug \"zzz\""), "{msg}");
        assert!(msg.contains("acme/skills"), "{msg}");
        assert!(!msg.contains("download failed"), "{msg}");
    }

    #[test]
    fn fetch_selects_a_skill_under_a_dot_directory() {
        // Hidden directories (names starting with `.`) are scanned like any
        // other: a skill may legitimately live under `.agents/`.
        let body = tarball(
            "skills-main",
            &[(
                ".agents/pdf/SKILL.md",
                "---\nname: pdf\ndescription: hidden\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("a skill under a dot-directory should match");
        assert_eq!(skill.name, "pdf");
        assert_eq!(skill.description, "hidden");
        assert_eq!(skill.dir, tmp.path().join("skills/.agents/pdf"));
    }

    #[test]
    fn no_ref_uses_head() {
        let body = tarball(
            "skills-main",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, urls) = fake_get(&[FORM_HEAD], body);
        let (_tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");
        let urls = urls.lock().unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with(FORM_HEAD));
        assert!(urls[0].starts_with("https://codeload.github.com/acme/skills/"));
    }

    #[test]
    fn branch_ref_targets_refs_heads() {
        let body = tarball(
            "skills-main",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, urls) = fake_get(&["tar.gz/refs/heads/feat/x"], body);
        let (_tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", Some("feat/x"), &get)
            .unwrap()
            .expect("branch ref should match");
        let urls = urls.lock().unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with("tar.gz/refs/heads/feat/x"));
    }

    #[test]
    fn tag_ref_falls_back_from_heads_to_tags() {
        // Branches are tried first; a tag exists only under refs/tags.
        let body = tarball(
            "skills-v1.2",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, urls) = fake_get(&["tar.gz/refs/tags/v1.2"], body);
        let (_tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", Some("v1.2"), &get)
            .unwrap()
            .expect("tag ref should match");
        let urls = urls.lock().unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].ends_with("tar.gz/refs/heads/v1.2"));
        assert!(urls[1].ends_with("tar.gz/refs/tags/v1.2"));
    }

    #[test]
    fn full_sha_ref_is_used_as_is() {
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        let body = tarball(
            "skills-sha",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, urls) = fake_get(&[SHA], body);
        let (_tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", Some(SHA), &get)
            .unwrap()
            .expect("sha ref should match");
        let urls = urls.lock().unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with(&format!("tar.gz/{SHA}")));
    }

    #[test]
    fn unknown_repo_or_ref_is_a_clean_not_found_error() {
        let get = |url: &str| -> Result<Box<dyn Read>> {
            let _ = url;
            Err(SkillsError::Http(Box::new(ureq::Error::StatusCode(404))))
        };
        let e = fetch_skill_with("acme", "skills", "pdf", Some("main"), &get).unwrap_err();
        let msg = decorate_download_error(e, "acme/skills").to_string();
        assert!(msg.contains("not found"), "{msg}");
        assert!(msg.contains("acme/skills"), "{msg}");
        assert!(msg.contains("--ref"), "{msg}");
    }

    #[test]
    fn ref_forms_are_ordered_branch_then_tag() {
        assert_eq!(ref_forms(None), vec!["HEAD".to_string()]);
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(ref_forms(Some(SHA)), vec![SHA.to_string()]);
        assert_eq!(
            ref_forms(Some("v1")),
            vec!["refs/heads/v1".to_string(), "refs/tags/v1".to_string()]
        );
        // Abbreviated SHAs are not special-cased.
        assert_eq!(
            ref_forms(Some("0123abcd")),
            vec![
                "refs/heads/0123abcd".to_string(),
                "refs/tags/0123abcd".to_string()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_comes_from_the_tar_entry_mode() {
        use std::os::unix::fs::PermissionsExt;

        let body = tarball(
            "skills-main",
            &[
                (
                    "pdf/SKILL.md",
                    "---\nname: pdf\ndescription: d\n---\nbody",
                    0o644,
                ),
                ("pdf/scripts/run.sh", "#!/bin/sh\n", 0o755),
            ],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");

        let mode = |p: &str| {
            std::fs::metadata(tmp.path().join("skills").join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
        };
        assert_ne!(mode("pdf/scripts/run.sh"), 0, "script should be executable");
        assert_eq!(mode("pdf/SKILL.md"), 0, "manifest should not be executable");
    }

    #[test]
    fn symlink_entries_are_skipped() {
        let body = tarball(
            "skills-main",
            &[(
                "pdf/SKILL.md",
                "---\nname: pdf\ndescription: d\n---\nbody",
                0o644,
            )],
            &[("pdf/link", "/etc/passwd")],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
        let (tmp, _skill) = fetch_skill_with("acme", "skills", "pdf", None, &get)
            .unwrap()
            .expect("pdf should match");
        assert!(!tmp.path().join("skills-main/pdf/link").exists());
    }

    #[test]
    fn traversal_entries_cannot_escape_the_temp_dir() {
        // A hand-crafted tar whose entry path escapes the target dir: the file
        // must never land outside the unpack root (whatever tar decides —
        // refuse or skip).
        let mut raw = raw_tar_header("../evil.txt", b"pwned");
        raw.extend_from_slice(&[0u8; 1024]); // end-of-archive blocks
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        use std::io::Write;
        let mut gz = encoder;
        gz.write_all(&raw).unwrap();
        let bytes = gz.finish().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let _ = unpack_archive(&bytes[..], tmp.path());
        assert!(!tmp.path().parent().unwrap().join("evil.txt").exists());
    }

    /// A minimal valid ustar header block for a regular file entry.
    fn raw_tar_header(name: &str, content: &[u8]) -> Vec<u8> {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[100..108].copy_from_slice(b"0000644\0"); // mode
        block[108..116].copy_from_slice(b"0000000\0"); // uid
        block[116..124].copy_from_slice(b"0000000\0"); // gid
        block[124..136].copy_from_slice(format!("{:011o}\0", content.len()).as_bytes()); // size
        block[136..148].copy_from_slice(b"00000000000\0"); // mtime
        block[156] = b'0'; // regular file
        block[257..262].copy_from_slice(b"ustar");
        block[263..265].copy_from_slice(b"00");
        for b in &mut block[148..156] {
            *b = b' ';
        }
        let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
        block[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());

        let mut out = block.to_vec();
        out.extend_from_slice(content);
        let pad = (512 - content.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
        out
    }

    #[test]
    fn a_body_that_is_not_gzip_is_an_error() {
        let (get, _urls) = fake_get(&[FORM_HEAD], b"not a tarball".to_vec());
        assert!(
            fetch_skill_with("acme", "skills", "pdf", None, &get).is_err(),
            "garbage body must fail, not silently match nothing"
        );
    }

    #[test]
    fn deterministic_client_errors_are_not_retried() {
        let calls = Mutex::new(0);
        let get = |url: &str| -> Result<Box<dyn Read>> {
            let _ = url;
            *calls.lock().unwrap() += 1;
            Err(SkillsError::Http(Box::new(ureq::Error::StatusCode(404))))
        };
        assert!(
            fetch_skill_with("acme", "skills", "pdf", None, &get).is_err(),
            "fetch should propagate the 404"
        );
        assert_eq!(*calls.lock().unwrap(), 1, "a 404 must not be retried");
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
