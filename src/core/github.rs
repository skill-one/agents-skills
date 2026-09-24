//! Fetch exactly one named skill from a GitHub repository with a single request.
//!
//! The whole repository tarball is downloaded from `codeload.github.com` — the
//! GitHub REST API is never touched, so the anonymous 60-requests-per-hour API
//! rate limit does not apply. The archive is unpacked into a temp dir and the
//! skill is matched locally: a skill is a **directory** that directly contains
//! `SKILL.md` and whose directory name matches the requested name
//! (case-insensitive, shallowest match wins); a `SKILL.md` at the repository
//! root is selected by the repository name. Nothing but the single tarball
//! request is needed; public repositories only, and Git LFS files install as
//! their pointer stubs.
//!
//! The HTTP client is injected as a `get` closure so the whole selection logic is
//! unit-testable offline; production uses a plain ureq GET.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

use crate::core::discover::{Skill, read_description, read_skill};
use crate::error::{Result, SkillsError};

/// Injected HTTP GET: returns the response body of `url`.
type Get<'a> = &'a (dyn Fn(&str) -> Result<Vec<u8>> + Sync);

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
        Err(e) => Err(decorate_download_error(e, &slug)),
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
        match get(&archive_url(owner, repo, &form)) {
            Ok(bytes) => {
                unpack_archive(&bytes, root)?;
                return Ok(repo_root_of(root));
            }
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

/// Unpack a gzipped tar archive into `root`.
///
/// Only regular files are unpacked: directories are created on demand, and
/// symlinks, hardlinks, and metadata headers are never part of a skill.
/// `unpack_in` refuses entries whose path would escape `root`, so a crafted
/// archive cannot write outside the temp dir.
fn unpack_archive(bytes: &[u8], root: &Path) -> Result<()> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
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

/// The repository root inside the unpacked archive: GitHub tarballs hold one
/// top-level `<repo>-<ref>` directory, which is stripped.
fn repo_root_of(root: &Path) -> PathBuf {
    let Ok(entries) = fs::read_dir(root) else {
        return root.to_path_buf();
    };
    let dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    if dirs.len() == 1 && dirs[0].is_dir() {
        dirs[0].clone()
    } else {
        root.to_path_buf()
    }
}

/// The skill matching [`fetch_skill`] promises, run on the unpacked tree.
///
/// A directory that directly contains `SKILL.md` whose name matches
/// (case-insensitive) is a candidate; the shallowest wins, ties broken by path
/// for determinism. A root-level manifest has no directory name, so it takes
/// the repository name.
fn select_skill(repo_root: &Path, repo: &str, skill_name: &str) -> Result<Option<Skill>> {
    let mut candidates: Vec<(usize, String)> = Vec::new();
    collect_manifest_dirs(repo_root, "", &mut candidates)?;

    let mut matched: Option<(usize, String)> = None;
    for (depth, dir) in candidates {
        let name = if dir.is_empty() {
            repo
        } else {
            dir.rsplit('/').next().unwrap_or_default()
        };
        if !name.eq_ignore_ascii_case(skill_name) {
            continue;
        }
        let better = match &matched {
            None => true,
            Some((d, p)) => (depth, dir.as_str()) < (*d, p.as_str()),
        };
        if better {
            matched = Some((depth, dir));
        }
    }

    let Some((_, dir)) = matched else {
        return Ok(None);
    };
    // The matched directory is the skill; the manifest contributes only the
    // description. Explicit selection also makes internal skills visible.
    // The root case has no directory name, so the skill is named after the
    // repository.
    Ok(if dir.is_empty() {
        Some(Skill {
            name: repo.to_string(),
            description: read_description(&repo_root.join("SKILL.md")),
            dir: repo_root.to_path_buf(),
        })
    } else {
        read_skill(&repo_root.join(&dir), true)
    })
}

/// Every directory under `dir` (relative path `rel`) that directly contains a
/// `SKILL.md`. Hidden entries are never skills.
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
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
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
/// no skill directory matched the name.
fn fetch_skill_with(
    owner: &str,
    repo: &str,
    skill_name: &str,
    reference: Option<&str>,
    get: Get,
) -> Result<Option<(tempfile::TempDir, Skill)>> {
    let temp = tempfile::TempDir::new()?;
    let repo_root = download_repo(owner, repo, reference, temp.path(), get)?;
    match select_skill(&repo_root, repo, skill_name)? {
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
/// authentication: only public repositories are supported.
fn http_get(url: &str) -> Result<Vec<u8>> {
    let attempt = || -> Result<Vec<u8>> {
        let mut resp = agent()
            .get(url)
            .header("User-Agent", "agents-skills")
            .call()?;
        let mut buf = Vec::new();
        resp.body_mut().as_reader().read_to_end(&mut buf)?;
        Ok(buf)
    };
    with_retry(3, attempt)
}

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
    type FakeGet = Box<dyn Fn(&str) -> Result<Vec<u8>> + Send + Sync>;

    /// An injected `get` serving `body` for every URL ending in one of `forms`,
    /// and a 404 for everything else. Records the requested URLs in order.
    fn fake_get(forms: &[&str], body: Vec<u8>) -> (FakeGet, UrlLog) {
        let forms: Vec<String> = forms.iter().map(|f| (*f).to_string()).collect();
        let requested = std::sync::Arc::new(Mutex::new(Vec::new()));
        let urls = requested.clone();
        let get = move |url: &str| -> Result<Vec<u8>> {
            requested.lock().unwrap().push(url.to_string());
            if forms.iter().any(|f| url.ends_with(f.as_str())) {
                Ok(body.clone())
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
        let root = tmp.path().join("skills-main");
        assert!(root.join("pdf/SKILL.md").is_file());
        assert!(root.join("pdf/scripts/run.sh").is_file());
        // The whole repo is unpacked; only the *selection* is narrowed.
        assert!(root.join("skills/doc/SKILL.md").is_file());
        assert!(root.join("README.md").is_file());
        assert_eq!(skill.dir, root.join("pdf"));
    }

    #[test]
    fn fetch_matches_on_directory_name_ignoring_frontmatter_name() {
        // The skill name is the directory name; the frontmatter `name` is
        // ignored. Matching is case-insensitive.
        let body = tarball(
            "skills-main",
            &[(
                "skills/PDF Master/SKILL.md",
                "---\nname: pdf-skill\ndescription: d\n---\nbody",
                0o644,
            )],
            &[],
        );
        let (get, _urls) = fake_get(&[FORM_HEAD], body);
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
        // selecting it installs the whole repository.
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
        let (tmp, skill) = fetch_skill_with("acme", "skills", "skills", None, &get)
            .unwrap()
            .expect("root manifest is selected with the repo name");
        assert_eq!(skill.name, "skills");
        assert_eq!(skill.description, "root skill");
        let root = tmp.path().join("skills-main");
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
        let root = tmp.path().join("skills-main");
        assert!(root.join("pdf/SKILL.md").is_file());
        assert_eq!(skill.dir, root.join("pdf"));
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
        assert!(!msg.contains("download failed"), "{msg}");
    }

    #[test]
    fn no_ref_uses_head() {
        let body = tarball(
            "skills-main",
            &[("pdf/SKILL.md", "---\ndescription: d\n---\nbody", 0o644)],
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
            &[("pdf/SKILL.md", "---\ndescription: d\n---\nbody", 0o644)],
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
            &[("pdf/SKILL.md", "---\ndescription: d\n---\nbody", 0o644)],
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
            &[("pdf/SKILL.md", "---\ndescription: d\n---\nbody", 0o644)],
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
        let get = |url: &str| -> Result<Vec<u8>> {
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
            std::fs::metadata(tmp.path().join("skills-main").join(p))
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
        let _ = unpack_archive(&bytes, tmp.path());
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
        let get = |url: &str| -> Result<Vec<u8>> {
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
