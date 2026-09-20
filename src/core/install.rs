//! Install skills into the canonical dir and list what's installed.
//!
//! The canonical dir (`(global ? home : cwd)/.agents/skills`) is the single source of
//! truth: [`install_skill`] writes real files there and nowhere else. Agent
//! integration is a separate concern handled by [`crate::core::link`]. Copies skip
//! metadata.json/.git/__pycache__/__pypackages__.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{Env, canonical_skills_dir, disabled_skills_dir};
use crate::core::discover::{Skill, parse_skill_md};
use crate::error::Result;

/// Outcome of installing a single skill into the canonical dir.
#[derive(Debug)]
pub struct InstallResult {
    /// Whether the install succeeded.
    pub success: bool,
    /// Canonical directory of the skill.
    pub canonical_path: PathBuf,
    /// Whether the install was skipped (source already inside the canonical dir).
    pub skipped: bool,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Characters that can never appear in a slot name, whatever the filesystem: `/` and
/// `\` are path separators (nested dirs, traversal), and the rest are rejected by
/// Windows (`< > : " | ? *`) or confusing on macOS (`:`).
const UNSAFE_CHARS: [char; 9] = ['/', '\\', '<', '>', ':', '"', '|', '?', '*'];

/// Longest slot name, in **bytes** — the common filesystem limit (ext4 / APFS /
/// HFS+ all cap a single name at 255 bytes). Truncation must be byte-based and
/// land on a character boundary: 255 CJK characters are 765 bytes and would make
/// `create_dir` fail with `ENAMETOOLONG`.
const MAX_SLOT_BYTES: usize = 255;

/// Fold a skill name into its **slot name**: the directory the skill occupies in the
/// canonical (or disabled) dir.
///
/// lowercase → every unsafe character ([`UNSAFE_CHARS`], whitespace, control
/// characters) and every `-` collapses into a single `-` → leading/trailing `.` and
/// `-` are trimmed → truncated to [`MAX_SLOT_BYTES`] bytes at a character
/// boundary. Everything else is kept, including
/// non-ASCII letters and digits (`中文技能`) and punctuation that is legal in a file
/// name (`c#`, `c++`), so two different skill names practically never fold onto the
/// same slot.
///
/// A name the fold leaves empty — only punctuation or whitespace, e.g. `"***"` —
/// carries no identity, so it falls back to a digest of the original name
/// (`skill-3f9a2c1d`): deterministic across runs, and still distinct per name.
pub fn sanitize_name(name: &str) -> String {
    let mut folded = String::new();
    let mut prev_dash = false;
    for c in name.to_lowercase().chars() {
        if c == '-' || c.is_control() || c.is_whitespace() || UNSAFE_CHARS.contains(&c) {
            if !prev_dash {
                folded.push('-');
                prev_dash = true;
            }
            continue;
        }
        folded.push(c);
        prev_dash = false;
    }

    let trimmed = folded.trim_matches(|c: char| c == '.' || c == '-');
    // Truncate by bytes, never mid-character: the limit is a filesystem byte
    // limit, and a partial UTF-8 sequence is not a valid name.
    let mut slot = String::new();
    for c in trimmed.chars() {
        if slot.len() + c.len_utf8() > MAX_SLOT_BYTES {
            break;
        }
        slot.push(c);
    }
    if slot.is_empty() {
        slot = format!("skill-{}", short_digest(name));
    }
    slot
}

/// Stable 64-bit FNV-1a digest of `name`, rendered as eight hex digits — a
/// dependency-free way to keep the names that fold to nothing distinct.
fn short_digest(name: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash as u32)
}

/// Directories in `dir` that denote the same skill as `name`: their on-disk name
/// normalizes to the same string.
///
/// A raw name comparison is not enough — a skill adopted from an agent dir keeps
/// that dir's original name (`PDF Master`), which need not equal
/// `sanitize_name(frontmatter name)` (`pdf-master`). Dot-entries are never skills.
fn same_skill_entries(dir: &Path, name: &str) -> Vec<PathBuf> {
    let key = sanitize_name(name);
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            let raw = entry.file_name().to_string_lossy().into_owned();
            !raw.starts_with('.') && sanitize_name(&raw) == key && entry.path().is_dir()
        })
        .map(|entry| entry.path())
        .collect()
}

/// Whether `dir` already holds the skill `name`, under the normalized slot name or
/// under an adopted directory name that normalizes to the same skill.
fn holds_skill(dir: &Path, name: &str) -> bool {
    dir.join(sanitize_name(name)).symlink_metadata().is_ok()
        || !same_skill_entries(dir, name).is_empty()
}

/// Canonicalize as much of `p` as exists: the deepest existing ancestor is
/// canonicalized and the not-yet-created tail appended. Unlike
/// `Path::canonicalize`, this also succeeds for paths that do not exist yet,
/// so an existing base and a to-be-created target resolve against the same
/// symlink-resolved root instead of comparing absolute vs raw paths.
fn canonicalize_lenient(p: &Path) -> PathBuf {
    let mut tail = PathBuf::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(resolved) = cur.canonicalize() {
            return resolved.join(&tail);
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail = PathBuf::from(name).join(&tail);
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

fn path_safe(base: &Path, target: &Path) -> bool {
    let base_abs = canonicalize_lenient(base);
    let target_abs = canonicalize_lenient(target);
    target_abs == base_abs || target_abs.starts_with(&base_abs)
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    path_safe(a, b) || path_safe(b, a)
}

/// Recursively copy a directory, excluding metadata.json / .git / __pycache__ / __pypackages__,
/// dereferencing symlinks (copying target contents).
pub fn copy_directory(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let src_path = entry.path();
        let dest_path = dest.join(&name);

        let meta = entry.metadata()?;
        if meta.is_dir() {
            if name == ".git" || name == "__pycache__" || name == "__pypackages__" {
                continue;
            }
            copy_directory(&src_path, &dest_path)?;
        } else if meta.is_file() {
            if name == "metadata.json" {
                continue;
            }
            fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Install a single skill into the canonical dir (the only place real files live).
///
/// An already-installed skill — enabled or disabled — is **skipped**, never
/// overwritten: `add` only ever adds. Update an installed skill with
/// `remove` + `add`.
pub fn install_skill(skill: &Skill, env: &Env) -> InstallResult {
    let skill_name = sanitize_name(&skill.name);
    let canonical_base = canonical_skills_dir(env);
    let canonical_dir = canonical_base.join(&skill_name);

    if !path_safe(&canonical_base, &canonical_dir) {
        return InstallResult {
            success: false,
            canonical_path: canonical_dir,
            skipped: false,
            error: Some("Invalid skill name: potential path traversal detected".to_string()),
        };
    }

    // Source already inside the canonical dir → skip (avoid deleting the source).
    if paths_overlap(&skill.dir, &canonical_dir) {
        return InstallResult {
            success: true,
            canonical_path: canonical_dir,
            skipped: true,
            error: None,
        };
    }

    // Already installed, enabled or disabled → skip. Installing anyway would
    // silently discard local edits, and over a *disabled* skill it would leave a
    // duplicate copy behind (both dirs hold the same skill). Both dirs are matched
    // by normalized name, so an adopted copy under an unnormalized directory name
    // (e.g. `PDF Master` for `pdf-master`) counts as installed too.
    let disabled_base = disabled_skills_dir(env);
    if holds_skill(&canonical_base, &skill.name) || holds_skill(&disabled_base, &skill.name) {
        return InstallResult {
            success: true,
            canonical_path: canonical_dir,
            skipped: true,
            error: None,
        };
    }

    // Copy into a staging dir next to the destination (same filesystem), then
    // swap it in with a rename: agents see either nothing or the complete skill,
    // never a half-copied one. A failed copy leaves no trace.
    let staging = canonical_base.join(format!(".incoming-{skill_name}-{}", unique_suffix()));
    let install = (|| -> Result<()> {
        fs::create_dir_all(&staging)?;
        copy_directory(&skill.dir, &staging)?;
        fs::rename(&staging, &canonical_dir)?;
        Ok(())
    })();

    if let Err(e) = install {
        remove_path(&staging);
        return InstallResult {
            success: false,
            canonical_path: canonical_dir,
            skipped: false,
            error: Some(e.to_string()),
        };
    }

    InstallResult {
        success: true,
        canonical_path: canonical_dir,
        skipped: false,
        error: None,
    }
}

/// Unique suffix for the staging directory name (pid + nanos).
fn unique_suffix() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    nanos ^ (std::process::id() as u128)
}

/// Remove a file or directory (whatever is at the path), ignoring errors.
fn remove_path(p: &Path) {
    if p.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false) {
        let _ = fs::remove_dir_all(p);
    } else {
        let _ = fs::remove_file(p);
    }
}

/// An installed skill (used by list).
#[derive(Debug)]
pub struct InstalledSkill {
    /// Skill name.
    pub name: String,
    /// Skill description, collapsed onto a single line.
    pub description: String,
    /// Directory the skill currently lives in (canonical or disabled).
    pub canonical_path: PathBuf,
    /// The skill directory's creation time as Unix seconds, when the platform
    /// and filesystem record one.
    ///
    /// Approximate "when it landed on disk": exact for `add` installs, but a
    /// skill adopted from an agent dir keeps that dir's original time, and
    /// several Linux filesystems report no creation time at all.
    pub installed_at: Option<u64>,
}

/// Collapse whitespace so a frontmatter block scalar reads as one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A directory's creation time as Unix seconds, when the platform/filesystem
/// records one (macOS and Windows do; some Linux filesystems do not).
pub fn dir_created_secs(dir: &Path) -> Option<u64> {
    fs::metadata(dir)
        .and_then(|m| m.created())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// Scan the canonical dir, listing installed skills.
pub fn list_installed_skills(env: &Env) -> Vec<InstalledSkill> {
    list_skills_in(&canonical_skills_dir(env))
}

/// List skills parked in the disabled dir.
pub fn list_disabled_skills(env: &Env) -> Vec<InstalledSkill> {
    list_skills_in(&disabled_skills_dir(env))
}

/// Read every skill directory in `dir`.
///
/// Dot-entries are skipped: staging leftovers (`.incoming-*`) and the `.misc`
/// quarantine dir live in the same tree but are never skills.
fn list_skills_in(dir: &Path) -> Vec<InstalledSkill> {
    let mut out: Vec<InstalledSkill> = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let skill_dir = entry.path();
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Some(skill) = parse_skill_md(&skill_md) else {
            continue;
        };
        let description = one_line(&skill.description);
        out.push(InstalledSkill {
            name: skill.name,
            description,
            installed_at: dir_created_secs(&skill_dir),
            canonical_path: skill_dir,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Scan the canonical dir, collecting installed skill directory names.
pub fn scan_installed(env: &Env) -> Vec<String> {
    scan_names_in(&canonical_skills_dir(env))
}

/// Scan the disabled dir, collecting disabled skill directory names.
pub fn scan_disabled(env: &Env) -> Vec<String> {
    scan_names_in(&disabled_skills_dir(env))
}

/// Collect the subdirectory names of `dir`, skipping dot-entries (staging
/// leftovers like `.incoming-*` and the `.misc` quarantine dir are never skills).
fn scan_names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.path().is_dir() {
                v.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    v.sort();
    v
}

/// Move a skill directory between the canonical dir and the disabled dir.
///
/// `to_enabled=true` moves `disabled-skills/<name>` → `skills/<name>` (enable);
/// `to_enabled=false` moves `skills/<name>` → `disabled-skills/<name>` (disable).
/// `name` is the **on-disk directory name** (as produced by [`scan_installed`]) and
/// is used as-is, so adopted skills whose name was never normalized still move.
/// The target parent dir is created if needed.
///
/// The copy being moved wins: a stale copy already occupying the target slot is
/// discarded first, so one skill name always maps to exactly one directory. A copy
/// under a differently normalized directory name (`PDF Master` vs `pdf-master`)
/// counts as the same skill and is discarded as well. This is what makes
/// `enable`/`disable` converge when a third-party agent re-installed a skill that
/// was still parked in the disabled dir.
pub fn move_skill(name: &str, to_enabled: bool, env: &Env) -> Result<()> {
    let canonical = canonical_skills_dir(env).join(name);
    let disabled = disabled_skills_dir(env).join(name);
    let (from, to) = if to_enabled {
        (&disabled, &canonical)
    } else {
        (&canonical, &disabled)
    };
    // Never free the target before the source is known to exist: a missing source
    // would turn the move into a plain delete.
    fs::symlink_metadata(from)?;
    let Some(parent) = to.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    // Free the target slot, dropping the stale copy that occupies it.
    for stale in same_skill_entries(parent, name) {
        remove_path(&stale);
    }
    if to.symlink_metadata().is_ok() {
        remove_path(to);
    }
    fs::rename(from, to)?;
    Ok(())
}

/// Delete every on-disk copy of the skill `name` — in both dirs, under the
/// normalized slot name or an adopted directory name that normalizes to the same
/// skill. Returns whether anything was actually removed.
pub fn remove_skill(name: &str, env: &Env) -> bool {
    let mut removed = false;
    for dir in [canonical_skills_dir(env), disabled_skills_dir(env)] {
        let mut slots = same_skill_entries(&dir, name);
        // The slot itself may hold a non-directory (a stray file), which the scan
        // above does not report but a rename would still collide with.
        slots.push(dir.join(name));
        for path in slots {
            if path.symlink_metadata().is_err() {
                continue;
            }
            remove_path(&path);
            removed |= path.symlink_metadata().is_err();
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_utils::{env_at, skill_frontmatter, write_and_parse_skill};

    fn write_skill(dir: &Path, name: &str) -> Skill {
        write_and_parse_skill(dir, name)
    }

    #[test]
    fn sanitize_name_basic() {
        assert_eq!(sanitize_name("PDF Master"), "pdf-master");
        assert_eq!(
            sanitize_name("Git Review Before Commit"),
            "git-review-before-commit"
        );
        assert_eq!(sanitize_name("../evil"), "evil");
        assert_eq!(sanitize_name("A.B_c"), "a.b_c");
        assert_eq!(sanitize_name("-leading-trailing-"), "leading-trailing");
    }

    #[test]
    fn sanitize_name_keeps_non_ascii_and_legal_punctuation() {
        // A non-ASCII name must stay identifiable: folding it to a shared placeholder
        // used to make every such skill collide on one slot.
        assert_eq!(sanitize_name("中文技能"), "中文技能");
        assert_ne!(sanitize_name("中文技能"), sanitize_name("另一技能"));
        assert_eq!(sanitize_name("C#"), "c#");
        assert_eq!(sanitize_name("C++"), "c++");
        assert_ne!(sanitize_name("C#"), sanitize_name("C++"));
        // Only what a file name cannot hold is folded away.
        assert_eq!(sanitize_name("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_name("a:b*c?d"), "a-b-c-d");
    }

    #[test]
    fn sanitize_name_falls_back_to_a_deterministic_digest() {
        // Nothing left to identify the skill by: the slot is still stable per name,
        // and two different names do not end up sharing one.
        let blank = sanitize_name("  ");
        assert!(blank.starts_with("skill-"), "got {blank}");
        assert_eq!(blank, sanitize_name("  "));
        assert_ne!(blank, sanitize_name("***"));
        assert_ne!(blank, sanitize_name(""));
    }

    #[test]
    fn sanitize_name_truncates_long_non_ascii_names_by_bytes() {
        // 255 is a byte limit, not a character count: 300 CJK characters are 900
        // bytes, so a character-based truncation would produce a name the
        // filesystem rejects with ENAMETOOLONG.
        let long: String = "技".repeat(300);
        let slot = sanitize_name(&long);
        assert!(slot.len() <= MAX_SLOT_BYTES, "{} bytes", slot.len());
        assert_eq!(slot.chars().count(), 85); // 85 * 3 bytes = 255 exactly
        assert!(slot.chars().all(|c| c == '技'));
    }

    #[test]
    fn path_safe_resolves_not_yet_created_targets_consistently() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path().join("skills");
        std::fs::create_dir_all(&base).unwrap();

        // Existing target: both sides canonicalize directly.
        let existing = base.join("alpha");
        std::fs::create_dir_all(&existing).unwrap();
        assert!(path_safe(&base, &existing));

        // Not-yet-created target under an existing base: the base canonicalizes
        // to an absolute path while the target cannot — the naive fallback made
        // this comparison fail (absolute vs raw) and reject a valid name.
        assert!(path_safe(&base, &base.join("beta")));

        // Fully not-yet-created base and target still resolve to one root.
        assert!(path_safe(
            &tmp.path().join("a/b"),
            &tmp.path().join("a/b/c")
        ));

        // Traversal outside the base is still rejected.
        assert!(!path_safe(&base, &tmp.path().join("elsewhere")));
    }

    #[test]
    fn install_skill_writes_canonical_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");

        let r = install_skill(&skill, &env);
        assert!(r.success, "err={:?}", r.error);
        assert!(!r.skipped);
        assert!(tmp.path().join(".agents/skills/pdf/SKILL.md").exists());
        // No agent dirs are created by install.
        assert!(!tmp.path().join(".claude").exists());
        assert!(!tmp.path().join(".windsurf").exists());
    }

    #[test]
    fn install_skill_skips_when_source_overlaps_canonical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join(".agents/skills/pdf");
        let skill = write_skill(&src, "pdf");

        let r = install_skill(&skill, &env);
        assert!(r.success);
        assert!(r.skipped);
        assert!(src.join("SKILL.md").exists());
    }

    #[test]
    fn install_skill_skips_when_already_installed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical_base = tmp.path().join(".agents/skills");
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");

        assert!(install_skill(&skill, &env).success);

        // A second install with changed content is skipped, leaving v1 intact.
        fs::write(src.join("SKILL.md"), "v2").unwrap();
        let r = install_skill(&write_skill(&src, "pdf"), &env);
        assert!(r.success);
        assert!(r.skipped);
        assert_eq!(
            fs::read_to_string(canonical_base.join("pdf/SKILL.md")).unwrap(),
            skill_frontmatter("pdf")
        );

        // No staging dir survives.
        let leftovers: Vec<_> = fs::read_dir(&canonical_base)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".incoming-"))
            .collect();
        assert!(leftovers.is_empty(), "leftovers: {leftovers:?}");
    }

    #[test]
    fn install_skill_skips_when_disabled() {
        // A disabled skill is still installed: installing it again must not drop
        // a second copy into the canonical dir.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");

        assert!(install_skill(&skill, &env).success);
        move_skill("pdf", false, &env).unwrap();

        let r = install_skill(&skill, &env);
        assert!(r.success);
        assert!(r.skipped);
        assert!(!tmp.path().join(".agents/skills/pdf").exists());
        assert!(
            tmp.path()
                .join(".agents/disabled-skills/pdf/SKILL.md")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_skill_failure_leaves_no_trace() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical_base = tmp.path().join(".agents/skills");
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");

        // An unreadable file makes fs::copy fail mid-install.
        let bad = src.join("bad.txt");
        fs::write(&bad, "x").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();

        let r = install_skill(&skill, &env);
        assert!(!r.success, "copy should have failed");

        // Nothing is left behind: neither the skill dir nor a staging dir.
        assert!(!canonical_base.join("pdf").exists());
        let leftovers: Vec<_> = fs::read_dir(&canonical_base)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".incoming-"))
            .collect();
        assert!(leftovers.is_empty(), "leftovers: {leftovers:?}");
    }

    #[test]
    fn move_skill_keeps_unnormalized_dir_name() {
        // Adopted skills keep their original dir name, which need not equal
        // sanitize_name(frontmatter name); disable/enable must still find them.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let dir = tmp.path().join(".agents/skills/PDF Master");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "x").unwrap();

        move_skill("PDF Master", false, &env).unwrap();
        assert!(!dir.exists());
        let parked = tmp
            .path()
            .join(".agents/disabled-skills/PDF Master/SKILL.md");
        assert!(parked.exists());

        move_skill("PDF Master", true, &env).unwrap();
        assert!(dir.join("SKILL.md").exists());
        assert!(
            !tmp.path()
                .join(".agents/disabled-skills/PDF Master")
                .exists()
        );
    }

    #[test]
    fn move_skill_discards_the_copy_occupying_the_target_slot() {
        // A disabled skill re-installed by a third-party agent: the same name now
        // lives in both dirs, and the copy being moved wins.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical = tmp.path().join(".agents/skills/pdf");
        let parked = tmp.path().join(".agents/disabled-skills/pdf");
        for (dir, body) in [(&parked, "parked"), (&canonical, "reinstalled")] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("SKILL.md"), body).unwrap();
        }

        move_skill("pdf", true, &env).unwrap();

        assert_eq!(
            fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "parked"
        );
        assert!(!parked.exists());
    }

    #[test]
    fn move_skill_discards_a_differently_named_copy_of_the_same_skill() {
        // An adopted dir keeps its original name, so `PDF Master` and `pdf-master`
        // are the same skill: moving one in must not leave two copies behind.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let adopted = tmp.path().join(".agents/skills/PDF Master");
        let parked = tmp.path().join(".agents/disabled-skills/pdf-master");
        for (dir, body) in [(&adopted, "adopted"), (&parked, "parked")] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("SKILL.md"), body).unwrap();
        }

        move_skill("pdf-master", true, &env).unwrap();

        assert!(!adopted.exists(), "the stale canonical copy must be gone");
        assert_eq!(
            fs::read_to_string(tmp.path().join(".agents/skills/pdf-master/SKILL.md")).unwrap(),
            "parked"
        );
        assert!(!parked.exists());
    }

    #[test]
    fn move_skill_keeps_the_target_when_the_source_is_gone() {
        // Freeing the target before the source is known to exist would turn the move
        // into a plain delete.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical = tmp.path().join(".agents/skills/pdf");
        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join("SKILL.md"), "enabled").unwrap();

        assert!(move_skill("pdf", true, &env).is_err());
        assert_eq!(
            fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "enabled"
        );
    }

    #[test]
    fn install_skill_skips_a_disabled_copy_under_an_unnormalized_name() {
        // The guard matches on the normalized name, so a parked `PDF Master` still
        // counts as installed — `add` must not create a second copy that
        // `enable`/`disable` would then have to resolve.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let parked = tmp.path().join(".agents/disabled-skills/PDF Master");
        fs::create_dir_all(&parked).unwrap();
        fs::write(parked.join("SKILL.md"), "parked").unwrap();

        let src = tmp.path().join("src-skill");
        let r = install_skill(&write_skill(&src, "PDF Master"), &env);

        assert!(r.success);
        assert!(r.skipped);
        assert!(!tmp.path().join(".agents/skills/pdf-master").exists());
    }

    #[test]
    fn install_skill_keeps_non_ascii_names_on_separate_slots() {
        // Two Chinese-named skills from one source: the second must land on its own
        // slot instead of being reported as a copy of the first.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let first = write_skill(&tmp.path().join("first"), "中文技能");
        let second = write_skill(&tmp.path().join("second"), "另一技能");

        assert!(install_skill(&first, &env).success);
        let r = install_skill(&second, &env);
        assert!(r.success, "err={:?}", r.error);
        assert!(!r.skipped, "the second Chinese-named skill must install");

        let base = tmp.path().join(".agents/skills");
        assert!(base.join("中文技能/SKILL.md").exists());
        assert!(base.join("另一技能/SKILL.md").exists());
    }

    #[test]
    fn remove_skill_deletes_both_copies_under_either_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical = tmp.path().join(".agents/skills/pdf-master");
        let parked = tmp.path().join(".agents/disabled-skills/PDF Master");
        for dir in [&canonical, &parked] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("SKILL.md"), "x").unwrap();
        }

        assert!(remove_skill("pdf-master", &env));
        assert!(!canonical.exists());
        assert!(!parked.exists());

        // Nothing left: the report must not claim a removal.
        assert!(!remove_skill("pdf-master", &env));
    }

    #[test]
    fn copy_directory_excludes_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::create_dir_all(src.join("__pycache__")).unwrap();
        fs::write(src.join("SKILL.md"), "x").unwrap();
        fs::write(src.join("metadata.json"), "x").unwrap();
        fs::write(src.join(".git").join("HEAD"), "x").unwrap();

        let dest = tmp.path().join("dest");
        copy_directory(&src, &dest).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(!dest.join("metadata.json").exists());
        assert!(!dest.join(".git").exists());
        assert!(!dest.join("__pycache__").exists());
    }

    #[test]
    fn list_installed_skills_finds_canonical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env);

        let installed = list_installed_skills(&env);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "pdf");
    }

    #[test]
    fn scan_installed_lists_canonical_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env);

        let names = scan_installed(&env);
        assert_eq!(names, vec!["pdf".to_string()]);
    }

    #[test]
    fn move_skill_disables_then_enables_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env);

        // Disable: moves out of canonical, into disabled-skills.
        move_skill("pdf", false, &env).unwrap();
        assert!(!tmp.path().join(".agents/skills/pdf").exists());
        assert!(
            tmp.path()
                .join(".agents/disabled-skills/pdf/SKILL.md")
                .exists()
        );
        assert!(scan_installed(&env).is_empty());
        assert_eq!(scan_disabled(&env), vec!["pdf".to_string()]);

        // Enable: moves back.
        move_skill("pdf", true, &env).unwrap();
        assert!(tmp.path().join(".agents/skills/pdf/SKILL.md").exists());
        assert!(scan_disabled(&env).is_empty());
    }

    #[test]
    fn list_disabled_skills_reports_hidden_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("src-skill");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env);
        move_skill("pdf", false, &env).unwrap();

        let disabled = list_disabled_skills(&env);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].name, "pdf");
    }
}
