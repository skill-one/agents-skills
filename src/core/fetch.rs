//! Fetching sources: shallow git clone + HTTP download + archive extraction.
//!
//! All functions return `tempfile::TempDir`; callers hold it until install finishes
//! (TempDir drops and cleans up automatically).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use git2::FetchOptions;
use git2::build::{CheckoutBuilder, RepoBuilder};

use crate::core::source::{Source, SourceType};
use crate::error::{Result, SkillsError};

/// Shared HTTP agent: honors `HTTP(S)_PROXY` / `ALL_PROXY` / `NO_PROXY` env vars
/// (ureq reads them via `Proxy::try_from_env`) so proxied networks can reach GitHub.
pub(crate) fn agent() -> &'static ureq::Agent {
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
pub(crate) fn with_retry<T>(attempts: usize, mut f: impl FnMut() -> Result<T>) -> Result<T> {
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

/// Fetch a non-local source into a temp dir, returning `(temp, root path)`.
///
/// GitHub / GitLab sources download the whole-repo archive (codeload / GitLab
/// `/-/archive`) instead of a git clone; only generic git / SSH URLs and GitLab
/// sources without a resolvable ref fall back to a shallow clone. The caller
/// holds the returned `TempDir` until install finishes.
pub fn fetch_source(parsed: &Source) -> Result<(tempfile::TempDir, std::path::PathBuf)> {
    match parsed.ty {
        SourceType::Github | SourceType::Gitlab => {
            if let Some(url) = parsed.archive_url() {
                return download_and_extract(&url);
            }
            let tmp = clone_repo(&parsed.url, parsed.r#ref.as_deref())?;
            let root = tmp.path().to_path_buf();
            Ok((tmp, root))
        }
        SourceType::Download => download_and_extract(&parsed.url),
        SourceType::Git => {
            let tmp = clone_repo(&parsed.url, parsed.r#ref.as_deref())?;
            let root = tmp.path().to_path_buf();
            Ok((tmp, root))
        }
        SourceType::Local => Err(SkillsError::msg(
            "local sources are handled by the caller, not fetch_source",
        )),
    }
}

/// Shallow-clone a git repo into a temp dir; checkout `reference` when it is a branch/tag.
pub fn clone_repo(url: &str, reference: Option<&str>) -> Result<tempfile::TempDir> {
    // Each attempt uses a fresh temp dir: a partial clone leaves a non-empty dir
    // that a retry could not clone into.
    let attempt = || -> Result<tempfile::TempDir> {
        let tmp = tempfile::TempDir::new()?;
        let mut builder = RepoBuilder::new();
        let mut fetch_opts = FetchOptions::new();
        fetch_opts.depth(1);
        builder.fetch_options(fetch_opts);
        if let Some(r) = reference {
            builder.branch(r);
        }
        // The local file:// transport does not support shallow fetch; fall back to non-shallow on failure.
        if builder.clone(url, tmp.path()).is_err() {
            let mut builder = RepoBuilder::new();
            if let Some(r) = reference {
                builder.branch(r);
            }
            builder.clone(url, tmp.path())?;
        }

        // After a shallow clone the branch may not be explicitly checked out; ensure the working tree is usable.
        if let Ok(repo) = git2::Repository::open(tmp.path())
            && let Some(r) = reference
        {
            let _ = checkout_ref(&repo, r);
        }
        Ok(tmp)
    };
    with_retry(3, attempt)
}

fn checkout_ref(repo: &git2::Repository, reference: &str) -> Result<()> {
    let mut builder = CheckoutBuilder::new();
    if let Ok(obj) = repo.revparse_single(reference) {
        repo.checkout_tree(&obj, Some(&mut builder))?;
        let _ = repo.set_head_detached(obj.id());
    }
    Ok(())
}

/// Download a URL to a temp file, returning its path and temp dir.
fn download_to_file(url: &str) -> Result<(tempfile::TempDir, PathBuf)> {
    let tmp = tempfile::TempDir::new()?;
    let file = tmp.path().join("download");
    let attempt = || -> Result<()> {
        let mut resp = agent().get(url).call()?;
        let mut reader = resp.body_mut().as_reader();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        std::fs::write(&file, &buf)?;
        Ok(())
    };
    with_retry(3, attempt)?;
    Ok((tmp, file))
}

/// Download a URL and extract it into a temp dir, returning the extraction root.
///
/// Supports zip / tar / tar.gz / tgz; single files (e.g. SKILL.md) are written as-is.
pub fn download_and_extract(url: &str) -> Result<(tempfile::TempDir, PathBuf)> {
    let (_file_tmp, file) = download_to_file(url)?;
    let out = tempfile::TempDir::new()?;
    let root = out.path().to_path_buf();

    match detect_archive_kind(url, &file) {
        Some(ArchiveKind::Zip) => extract_zip(&file, &root)?,
        Some(ArchiveKind::TarGz) => extract_tar(&file, &root, true)?,
        Some(ArchiveKind::Tar) => extract_tar(&file, &root, false)?,
        // Single file: copy under root, preserving the original filename.
        None => {
            let name = file_name_from_url(url);
            std::fs::copy(&file, root.join(name))?;
        }
    }
    Ok((out, root))
}

fn file_name_from_url(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("SKILL.md");
    if name.is_empty() {
        "SKILL.md".to_string()
    } else {
        name.to_string()
    }
}

/// Archive format of a downloaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    /// ZIP archive.
    Zip,
    /// Uncompressed tar.
    Tar,
    /// Gzip-compressed tar.
    TarGz,
}

/// Decide what a downloaded file is: an archive (and which kind) or a single file.
///
/// ZIP is detected by magic bytes first (codeload serves zip regardless of URL);
/// otherwise the URL extension decides. `None` = single file, copied as-is.
fn detect_archive_kind(url: &str, file: &Path) -> Option<ArchiveKind> {
    if has_zip_magic(file) {
        return Some(ArchiveKind::Zip);
    }
    let path = url.split('?').next().unwrap_or(url).to_lowercase();
    if path.ends_with(".zip") {
        return Some(ArchiveKind::Zip);
    }
    if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        return Some(ArchiveKind::TarGz);
    }
    if path.ends_with(".tar") {
        return Some(ArchiveKind::Tar);
    }
    None
}

fn has_zip_magic(file: &Path) -> bool {
    std::fs::read(file)
        .map(|b| b.len() >= 2 && b[0] == b'P' && b[1] == b'K')
        .unwrap_or(false)
}

/// Extract a zip into the destination, rejecting `..` path traversal.
fn extract_zip(file: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(file)?;
    let mut archive = zip::ZipArchive::new(f)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(name) = entry.enclosed_name() else {
            return Err(SkillsError::msg("Archive contains an unsafe path"));
        };
        let out = safe_join(dest, &name)?;
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }
    Ok(())
}

/// Extract a tar (optionally gzip-compressed) into the destination.
fn extract_tar(file: &Path, dest: &Path, gz: bool) -> Result<()> {
    let f = std::fs::File::open(file)?;
    let reader: Box<dyn std::io::Read> = if gz {
        Box::new(flate2::read::GzDecoder::new(f))
    } else {
        Box::new(f)
    };
    let mut archive = tar::Archive::new(reader);
    unpack_tar(&mut archive, dest)?;
    Ok(())
}

fn unpack_tar<R: std::io::Read>(archive: &mut tar::Archive<R>, dest: &Path) -> Result<()> {
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let out = safe_join(dest, &path)?;
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&out)?;
        }
    }
    Ok(())
}

/// Reject `..` segments and absolute paths so the extraction target stays within `dest`.
fn safe_join(dest: &Path, rel: &Path) -> Result<PathBuf> {
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(SkillsError::msg("Archive contains an unsafe path"));
    }
    Ok(dest.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_local_repo_via_file_url() {
        // Create a local repo with git2 and commit a SKILL.md.
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let repo = git2::Repository::init(&repo_dir).unwrap();
        std::fs::write(
            repo_dir.join("SKILL.md"),
            "---\nname: test\ndescription: test skill\n---\nbody\n",
        )
        .unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("SKILL.md")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let url = format!("file://{}", repo_dir.display());
        let cloned = clone_repo(&url, None).unwrap();
        assert!(cloned.path().join("SKILL.md").exists());
    }

    #[test]
    fn download_single_file() {
        // A local file:// cannot be handled directly by ureq; this only verifies the helper for single-file copying.
        assert_eq!(file_name_from_url("https://x/y/SKILL.md?a=b"), "SKILL.md");
        assert_eq!(file_name_from_url("https://x/y/"), "SKILL.md");
    }

    #[test]
    fn fetch_source_rejects_local() {
        // Local sources are handled inline by the caller (manager).
        let s = crate::core::source::parse_source("./x").unwrap();
        assert!(matches!(fetch_source(&s), Err(SkillsError::Message(_))));
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

    #[test]
    fn safe_join_rejects_traversal() {
        let dest = Path::new("/tmp/x");
        assert!(safe_join(dest, Path::new("../evil")).is_err());
        assert!(safe_join(dest, Path::new("/abs")).is_err());
        assert!(safe_join(dest, Path::new("a/b/c")).is_ok());
    }

    #[test]
    fn extract_zip_with_nested_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let zip_path = tmp.path().join("skill.zip");
        let out = tmp.path().join("out");

        // Build a zip: skill/SKILL.md + skill/scripts/run.sh
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("skill/SKILL.md", opts).unwrap();
            std::io::Write::write_all(&mut zw, b"---\nname: pdf\n---\n").unwrap();
            zw.start_file("skill/scripts/run.sh", opts).unwrap();
            std::io::Write::write_all(&mut zw, b"#!/bin/sh\n").unwrap();
            zw.finish().unwrap();
        }

        extract_zip(&zip_path, &out).unwrap();
        assert!(out.join("skill/SKILL.md").exists());
        assert!(out.join("skill/scripts/run.sh").exists());
    }

    #[test]
    fn extract_tar_gz_with_nested_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tar_path = tmp.path().join("skill.tar.gz");
        let out = tmp.path().join("out");

        {
            let f = std::fs::File::create(&tar_path).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut tw = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(20);
            header.set_mode(0o644);
            header.set_cksum();
            tw.append_data(
                &mut header,
                "skill/SKILL.md",
                b"---\nname: pdf\n---\n".as_slice(),
            )
            .unwrap();
            tw.finish().unwrap();
        }

        extract_tar(&tar_path, &out, true).unwrap();
        assert!(out.join("skill/SKILL.md").exists());
    }

    #[test]
    fn extract_plain_tar_with_nested_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tar_path = tmp.path().join("skill.tar");
        let out = tmp.path().join("out");

        {
            let f = std::fs::File::create(&tar_path).unwrap();
            let mut tw = tar::Builder::new(f);
            let mut header = tar::Header::new_gnu();
            header.set_size(20);
            header.set_mode(0o644);
            header.set_cksum();
            tw.append_data(
                &mut header,
                "skill/SKILL.md",
                b"---\nname: pdf\n---\n".as_slice(),
            )
            .unwrap();
            tw.finish().unwrap();
        }

        extract_tar(&tar_path, &out, false).unwrap();
        assert!(out.join("skill/SKILL.md").exists());
    }

    #[test]
    fn extract_zip_rejects_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("../evil.txt", opts).unwrap();
            std::io::Write::write_all(&mut zw, b"evil").unwrap();
            zw.finish().unwrap();
        }

        assert!(extract_zip(&zip_path, &out).is_err());
    }
}
