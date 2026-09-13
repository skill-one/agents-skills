//! Read/write `skills-lock.json` and hash skill directory contents.
//!
//! Format: `{ version: 1, skills: { name: { source, sourceUrl, ref, sourceType, skillPath, computedHash } } }`.
//! Writes sort keys to guarantee deterministic output and fewer merge conflicts.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::locale;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::install::sanitize_name;
use crate::core::source::{Source, SourceType, owner_repo};
use crate::error::Result;

/// Lock file name (`skills-lock.json`).
pub const LOCAL_LOCK_FILE: &str = "skills-lock.json";
/// Current lock file format version.
pub const CURRENT_VERSION: u32 = 1;

/// The `skills-lock.json` root structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalLockFile {
    /// Lock format version.
    pub version: u32,
    /// Installed skills keyed by (sanitized) name.
    pub skills: BTreeMap<String, LockEntry>,
}

/// A single skill's lock metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockEntry {
    /// Source identifier: owner/repo, local path, URL, etc.
    pub source: String,
    /// Resolved source URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Branch or tag used at install time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    /// Source type (github / gitlab / git / local / download).
    pub source_type: String,
    /// Skill path within the source repo (e.g. `skills/pdf`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
    /// SHA-256 of directory contents.
    pub computed_hash: String,
}

impl LockEntry {
    /// Construct a lock entry with the given source and content hash.
    pub fn new(source: &str, source_type: &str, hash: String) -> Self {
        LockEntry {
            source: source.to_string(),
            source_url: None,
            r#ref: None,
            source_type: source_type.to_string(),
            skill_path: None,
            computed_hash: hash,
        }
    }
}

/// Find a lock entry by raw or sanitized name.
pub fn find_lock_entry<'a>(lock: &'a LocalLockFile, name: &str) -> Option<&'a LockEntry> {
    if let Some(e) = lock.skills.get(name) {
        return Some(e);
    }
    let san = sanitize_name(name);
    lock.skills
        .iter()
        .find(|(k, _)| sanitize_name(k) == san)
        .map(|(_, v)| v)
}

/// Compute a lock entry's source / sourceType / sourceUrl / ref / skillPath from a parsed source.
pub fn lock_fields(
    parsed: &Source,
) -> (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let source_type = match parsed.ty {
        SourceType::Github => "github",
        SourceType::Gitlab => "gitlab",
        SourceType::Git => "git",
        SourceType::Local => "local",
        SourceType::Download => "download",
    };
    match parsed.ty {
        SourceType::Local => (
            parsed
                .local_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            source_type.to_string(),
            None,
            parsed.r#ref.clone(),
            None,
        ),
        SourceType::Github => (
            owner_repo(&parsed.url),
            source_type.to_string(),
            None,
            parsed.r#ref.clone(),
            parsed.subpath.clone(),
        ),
        SourceType::Gitlab | SourceType::Git => (
            parsed.url.clone(),
            source_type.to_string(),
            Some(parsed.url.clone()),
            parsed.r#ref.clone(),
            parsed.subpath.clone(),
        ),
        _ => (
            parsed.url.clone(),
            source_type.to_string(),
            None,
            parsed.r#ref.clone(),
            parsed.subpath.clone(),
        ),
    }
}

/// Read a lock file; return an empty structure if missing or corrupted.
pub fn read_local_lock(lock_path: &Path) -> LocalLockFile {
    let content = match fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(_) => return LocalLockFile::default(),
    };
    let mut lock: LocalLockFile = match serde_json::from_str(&content) {
        Ok(l) => l,
        Err(_) => return LocalLockFile::default(),
    };
    if lock.version < CURRENT_VERSION {
        return LocalLockFile::default();
    }
    // Resolve non-absolute local sources relative to the lock directory.
    let lock_dir = lock_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for entry in lock.skills.values_mut() {
        if entry.source_type == "local" {
            let p = PathBuf::from(&entry.source);
            if !p.is_absolute() {
                entry.source = lock_dir.join(p).to_string_lossy().into_owned();
            }
        }
    }
    lock
}

/// Write a lock file: sort skills by key and convert local sources to relative paths.
pub fn write_local_lock(lock: &LocalLockFile, lock_path: &Path) -> Result<()> {
    let lock_dir = lock_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut skills = BTreeMap::new();
    for (name, entry) in &lock.skills {
        let entry = if entry.source_type == "local" {
            let mut e = entry.clone();
            e.source = portable_local_source(&e.source, &lock_dir);
            e
        } else {
            entry.clone()
        };
        skills.insert(name.clone(), entry);
    }
    let out = LocalLockFile {
        version: lock.version,
        skills,
    };
    let json = serde_json::to_string_pretty(&out)?;
    fs::write(lock_path, format!("{json}\n"))?;
    Ok(())
}

/// Convert a local source to a cross-platform relative path (keep absolute on different drives).
fn portable_local_source(source: &str, lock_dir: &Path) -> String {
    let abs = PathBuf::from(source);
    let abs = if abs.is_absolute() {
        abs
    } else {
        lock_dir.join(abs)
    };
    let rel = pathdiff::diff_paths(&abs, lock_dir).unwrap_or(abs.clone());
    if rel.is_absolute() {
        return abs.to_string_lossy().replace('\\', "/");
    }
    let portable = rel.to_string_lossy().replace('\\', "/");
    if portable.is_empty() {
        return ".".to_string();
    }
    if portable == ".." || portable.starts_with("../") {
        return portable;
    }
    format!("./{portable}")
}

/// Compute SHA-256 of a skill directory, matching the upstream skills.sh hash:
/// for each file, append `utf8(relative path) + 0x00 + file bytes + 0x00` to a
/// single SHA-256 stream, with files sorted in case-insensitive path order
/// (ICU base-strength collation, i.e. `Intl.Collator("en", { sensitivity: "base" })`).
///
/// The NUL delimiters make path/content boundaries unambiguous, and paths
/// participate so renames are detected. `.git` and `node_modules` are skipped
/// (upstream snapshots never contain them either).
pub fn compute_folder_hash(skill_dir: &Path) -> Result<String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_files(skill_dir, skill_dir, &mut files)?;
    let collator = base_collator();
    files.sort_by(|a, b| collator.compare(&a.0, &b.0));

    let mut hasher = Sha256::new();
    for (rel, content) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0x00]);
        hasher.update(&content);
        hasher.update([0x00]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// ICU collator with base strength (ignores case and accents), matching
/// `Intl.Collator("en", { sensitivity: "base" })` used by the upstream hash.
fn base_collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR.get_or_init(|| {
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Primary);
        // `en` carries no collation keywords, so the strict parse always succeeds.
        let prefs = CollatorPreferences::from_locale_strict(&locale!("en"))
            .ok()
            .unwrap_or_default();
        Collator::try_new(prefs, options).expect("en collator is compiled in")
    })
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if name == ".git" || name == "node_modules" {
                continue;
            }
            collect_files(base, &path, out)?;
        } else if file_type.is_file() {
            let rel = pathdiff::diff_paths(&path, base)
                .unwrap_or_default()
                .to_string_lossy()
                .replace('\\', "/");
            let content = fs::read(&path)?;
            out.push((rel, content));
        }
    }
    Ok(())
}

/// Project lock file path.
pub fn local_lock_path(cwd: &Path) -> PathBuf {
    cwd.join(LOCAL_LOCK_FILE)
}

/// Global lock file path: `~/.agents/.skill-lock.json` (reuses the same structure).
pub fn global_lock_path(home: &Path) -> PathBuf {
    home.join(".agents").join(".skill-lock.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lock_reads_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lock = read_local_lock(&tmp.path().join(LOCAL_LOCK_FILE));
        assert_eq!(lock.version, 0);
        assert!(lock.skills.is_empty());
    }

    #[test]
    fn corrupted_lock_reads_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join(LOCAL_LOCK_FILE);
        fs::write(&p, "{ not valid json").unwrap();
        let lock = read_local_lock(&p);
        assert!(lock.skills.is_empty());
    }

    #[test]
    fn write_read_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join(LOCAL_LOCK_FILE);
        let mut lock = LocalLockFile {
            version: 1,
            skills: BTreeMap::new(),
        };
        lock.skills.insert(
            "pdf".to_string(),
            LockEntry {
                source: "acme/skills".to_string(),
                source_url: Some("https://github.com/acme/skills.git".to_string()),
                r#ref: Some("main".to_string()),
                source_type: "github".to_string(),
                skill_path: Some("skills/pdf".to_string()),
                computed_hash: "abc".to_string(),
            },
        );
        write_local_lock(&lock, &p).unwrap();

        let content = fs::read_to_string(&p).unwrap();
        // camelCase field names
        assert!(content.contains("\"sourceUrl\""));
        assert!(content.contains("\"skillPath\""));
        assert!(content.contains("\"computedHash\""));

        let read = read_local_lock(&p);
        assert_eq!(read.skills["pdf"].source, "acme/skills");
        assert_eq!(read.skills["pdf"].source_type, "github");
    }

    #[test]
    fn write_sorts_keys_alphabetically() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join(LOCAL_LOCK_FILE);
        let mut lock = LocalLockFile {
            version: 1,
            skills: BTreeMap::new(),
        };
        for name in ["zeta", "alpha", "mike"] {
            lock.skills.insert(
                name.to_string(),
                LockEntry::new("src", "github", "h".to_string()),
            );
        }
        write_local_lock(&lock, &p).unwrap();
        let content = fs::read_to_string(&p).unwrap();
        let alpha = content.find("\"alpha\"").unwrap();
        let mike = content.find("\"mike\"").unwrap();
        let zeta = content.find("\"zeta\"").unwrap();
        assert!(alpha < mike && mike < zeta, "keys must be sorted");
    }

    #[test]
    fn local_source_becomes_relative_on_write_and_absolute_on_read() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let p = tmp.path().join(LOCAL_LOCK_FILE);

        let mut lock = LocalLockFile {
            version: 1,
            skills: BTreeMap::new(),
        };
        lock.skills.insert(
            "s".to_string(),
            LockEntry::new(skill_dir.to_str().unwrap(), "local", "h".to_string()),
        );
        write_local_lock(&lock, &p).unwrap();
        let content = fs::read_to_string(&p).unwrap();
        assert!(
            content.contains("\"./skill\""),
            "should be portable relative path, got: {content}"
        );

        let read = read_local_lock(&p);
        let abs = PathBuf::from(&read.skills["s"].source);
        assert!(abs.is_absolute(), "read should resolve back to absolute");
    }

    #[test]
    fn folder_hash_is_deterministic_and_sensitive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("skill");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("SKILL.md"), "hello").unwrap();
        fs::write(dir.join("sub").join("a.txt"), "x").unwrap();

        let h1 = compute_folder_hash(&dir).unwrap();
        let h2 = compute_folder_hash(&dir).unwrap();
        assert_eq!(h1, h2, "same content -> same hash");

        // content change -> hash change
        fs::write(dir.join("SKILL.md"), "hello!").unwrap();
        let h3 = compute_folder_hash(&dir).unwrap();
        assert_ne!(h1, h3);

        // path change (rename) -> hash change
        fs::write(dir.join("SKILL.md"), "hello").unwrap();
        fs::rename(dir.join("sub").join("a.txt"), dir.join("sub").join("b.txt")).unwrap();
        let h4 = compute_folder_hash(&dir).unwrap();
        assert_ne!(h1, h4);
    }

    #[test]
    fn folder_hash_matches_upstream_spec() {
        // Known vector: sha256("SKILL.md" 0x00 "hello" 0x00).
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("SKILL.md"), "hello").unwrap();
        let mut expected = Sha256::new();
        expected.update(b"SKILL.md");
        expected.update([0x00]);
        expected.update(b"hello");
        expected.update([0x00]);
        assert_eq!(
            compute_folder_hash(dir.path()).unwrap(),
            format!("{:x}", expected.finalize())
        );
    }

    #[test]
    fn folder_hash_nul_delimiters_disambiguate_path_content() {
        // {"a" -> "b.txt"} vs {"a.b" -> "txt"} concatenate identically without
        // delimiters; NUL separators must keep them distinct.
        let mk = |name: &str, content: &str| {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(name), content).unwrap();
            let h = compute_folder_hash(dir.path()).unwrap();
            std::mem::forget(dir); // keep dirs alive until both hashes are taken
            h
        };
        assert_ne!(mk("a", "b.txt"), mk("a.b", "txt"));
    }

    #[test]
    fn folder_hash_sorts_case_insensitively() {
        // Base collation sorts "a.txt" before "B.txt"; plain byte order would
        // put "B.txt" first. Verify the stream follows collation order.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("B.txt"), "1").unwrap();
        fs::write(dir.path().join("a.txt"), "2").unwrap();

        let mut expected = Sha256::new();
        for (rel, content) in [("a.txt", "2"), ("B.txt", "1")] {
            expected.update(rel.as_bytes());
            expected.update([0x00]);
            expected.update(content.as_bytes());
            expected.update([0x00]);
        }
        assert_eq!(
            compute_folder_hash(dir.path()).unwrap(),
            format!("{:x}", expected.finalize())
        );
    }

    #[test]
    fn folder_hash_skips_git_and_node_modules() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("skill");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("SKILL.md"), "hello").unwrap();
        fs::write(dir.join(".git").join("HEAD"), "ref").unwrap();
        fs::write(dir.join("node_modules").join("x"), "x").unwrap();

        let dir2 = tmp.path().join("skill2");
        fs::create_dir_all(&dir2).unwrap();
        fs::write(dir2.join("SKILL.md"), "hello").unwrap();

        let h1 = compute_folder_hash(&dir).unwrap();
        let h2 = compute_folder_hash(&dir2).unwrap();
        assert_eq!(h1, h2, "ignored dirs must not affect hash");
    }
}
