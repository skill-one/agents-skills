//! Install skills into the canonical dir and scan what's installed.
//!
//! The canonical dir (`~/.agents/skills`) is the single source of truth:
//! [`install_skill`] writes real files there and nowhere else. Agent
//! integration is a separate concern handled by [`crate::core::link`]. Copies skip
//! metadata.json/.git/__pycache__/__pypackages__.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::agents::{Env, canonical_skills_dir, disabled_skills_dir};
use crate::core::discover::{Skill, read_skill};
use crate::core::path_util::path_contains;
use crate::error::{Result, SkillsError};

/// Outcome of installing a single skill into the canonical dir.
#[derive(Debug)]
pub struct InstallOutcome {
    /// Canonical directory of the skill.
    pub canonical_path: PathBuf,
    /// Whether the install was skipped (a skill of the same identity is
    /// already installed, or the source already lives inside the canonical dir).
    pub skipped: bool,
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

    let folded = folded.trim_matches(|c: char| c == '.' || c == '-');
    let mut slot = truncate_bytes(&windows_safe_slot(folded), MAX_SLOT_BYTES);
    if slot.is_empty() {
        slot = format!("skill-{}", short_digest(name));
    }
    slot
}

/// Truncate to `max` bytes, never mid-character: the limit is a filesystem
/// byte limit, and a partial UTF-8 sequence is not a valid name.
fn truncate_bytes(name: &str, max: usize) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
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

/// Windows reserves device names (`CON`, `AUX`, `COM1`, ...) case-insensitively,
/// with or without an extension, and Win32 silently strips trailing dots and
/// spaces — such slot names cannot round-trip on Windows. The adjustment is
/// deterministic, so every platform derives the same slot for the same name.
const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Adjust a slot name Windows would reject: trailing dots/spaces (stripped by
/// Win32 anyway) go away, and a reserved device base name gains a `_` suffix.
fn windows_safe_slot(name: &str) -> String {
    let trimmed = name.trim_end_matches(['.', ' ']);
    let base = trimmed.split('.').next().unwrap_or(trimmed);
    if WINDOWS_RESERVED.contains(&base.to_ascii_lowercase().as_str()) {
        format!("{trimmed}_")
    } else {
        trimmed.to_owned()
    }
}

/// Make a skill name addressable: lowercase, every space replaced by `-`,
/// `/` dropped, everything else kept verbatim (`&`, `.`, `_`, `:` survive) —
/// the slug rule used by skills.sh. The slug is the last segment of an
/// install id (`owner/repo/slug`); matching compares slugs, never raw names.
pub fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '/' => None,
            c => Some(c),
        })
        .collect()
}

/// Directories in `dir` that denote the same skill as `name`: their on-disk name
/// normalizes to the same string.
///
/// A raw name comparison is not enough — a skill adopted from an agent dir keeps
/// that dir's original name (`PDF Master`), which need not equal its normalized
/// slot (`pdf-master`). Dot-entries are never skills.
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

fn paths_overlap(a: &Path, b: &Path) -> bool {
    path_contains(a, b) || path_contains(b, a)
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
/// The installed directory keeps the source directory's own name — adjusted
/// only where Windows would reject it ([`windows_safe_slot`]) — while the
/// skill's identity (what `list` reports and
/// `remove`/`enable`/`disable` select by) is the slugified frontmatter
/// `name`. An already-installed skill — enabled or disabled — is **skipped**,
/// never overwritten: `add` only ever adds. Update an installed skill with
/// `remove` + `add`.
///
/// # Errors
///
/// [`SkillsError`] when the install slot name cannot be derived, exceeds the
/// filesystem's 255-byte name limit, or the copy into the canonical dir fails.
pub fn install_skill(skill: &Skill, env: &Env) -> Result<InstallOutcome> {
    let slug = slugify(&skill.name);
    let canonical_base = canonical_skills_dir(env);
    let slot = windows_safe_slot(skill.dir.file_name().and_then(|n| n.to_str()).ok_or_else(
        || {
            SkillsError::msg(format!(
                "Cannot determine an install directory name from \"{}\".",
                skill.dir.display()
            ))
        },
    )?);
    if slot.len() > MAX_SLOT_BYTES {
        return Err(SkillsError::msg(format!(
            "Install directory name \"{slot}\" exceeds the {MAX_SLOT_BYTES}-byte \
             filesystem limit; rename the source directory."
        )));
    }
    let canonical_dir = canonical_base.join(&slot);

    // Source already inside the canonical dir → skip (avoid deleting the source).
    if paths_overlap(&skill.dir, &canonical_dir) {
        return Ok(InstallOutcome {
            canonical_path: canonical_dir,
            skipped: true,
        });
    }

    // Already installed, enabled or disabled → skip. Installing anyway would
    // silently discard local edits, and over a *disabled* skill it would leave a
    // duplicate copy behind (both dirs hold the same skill). Both dirs are matched
    // by normalized name, so an adopted copy under an unnormalized directory name
    // (e.g. `PDF Master` for `pdf-master`) counts as installed too.
    let disabled_base = disabled_skills_dir(env);
    if holds_skill(&canonical_base, &slug) || holds_skill(&disabled_base, &slug) {
        return Ok(InstallOutcome {
            canonical_path: canonical_dir,
            skipped: true,
        });
    }

    // Copy into a staging dir next to the destination (same filesystem), then
    // swap it in with a rename: agents see either nothing or the complete skill,
    // never a half-copied one. A failed copy leaves no trace.
    let staging = canonical_base.join(format!(".incoming-{slot}-{}", unique_suffix()));
    let install = (|| -> Result<()> {
        fs::create_dir_all(&staging)?;
        copy_directory(&skill.dir, &staging)?;
        fs::rename(&staging, &canonical_dir)?;
        Ok(())
    })();

    if let Err(e) = install {
        remove_path(&staging);
        return Err(e);
    }

    Ok(InstallOutcome {
        canonical_path: canonical_dir,
        skipped: false,
    })
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

/// Remove a file, directory, or link (whatever is at the path), ignoring errors.
/// Links — junctions on Windows included — are removed as the link itself.
fn remove_path(p: &Path) {
    let Ok(meta) = p.symlink_metadata() else {
        return;
    };
    if meta.is_symlink() {
        let _ = crate::core::link::remove_link(p);
    } else if meta.is_dir() {
        let _ = fs::remove_dir_all(p);
    } else {
        let _ = fs::remove_file(p);
    }
}

/// A scanned skill directory: what the skill reports plus the slot it lives in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScannedSkill {
    /// Reported name — the slugified frontmatter `name`, the identity
    /// `remove`/`enable`/`disable` select by.
    pub name: String,
    /// The frontmatter `name` as declared — a display-only rendition of the
    /// same skill (it may contain spaces and mixed case the slug folds away).
    pub display_name: String,
    /// Skill description, collapsed onto a single line.
    pub description: String,
    /// Whether the skill declares `metadata.internal` (the scanners report
    /// it; visibility filtering is the caller's concern).
    pub internal: bool,
    /// The skill's real on-disk directory (canonical or disabled dir).
    pub path: PathBuf,
    /// The skill directory's creation time as Unix seconds, when the platform
    /// and filesystem record one. Approximate "when it landed on disk".
    pub installed_at: Option<u64>,
    /// On-disk directory name — the slot [`move_skill`] / [`remove_skill`]
    /// operate on.
    pub dir_name: String,
}

/// Scan every skill directory in `dir`, enabled or disabled.
///
/// Dot-entries are skipped: staging leftovers (`.incoming-*`) and the `.misc`
/// quarantine dir live in the same tree but are never skills. A directory is
/// only a skill when its SKILL.md declares a non-empty `name`; internal
/// skills are reported with their flag set — [`Manager::list`][list] filters
/// them, selection never does.
///
/// [list]: crate::manager::Manager::list
fn scan_skills_in(dir: &Path) -> Vec<ScannedSkill> {
    let mut out: Vec<ScannedSkill> = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !entry.path().is_dir() {
            continue;
        }
        let Some(skill) = read_skill(&entry.path()) else {
            continue;
        };
        out.push(ScannedSkill {
            name: slugify(&skill.name),
            display_name: skill.name,
            description: one_line(&skill.description),
            internal: skill.internal,
            path: entry.path(),
            installed_at: dir_created_secs(&entry.path()),
            dir_name: entry.file_name().to_string_lossy().into_owned(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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

/// Scan the canonical dir, collecting installed skills by reported name.
pub fn scan_installed(env: &Env) -> Vec<ScannedSkill> {
    scan_skills_in(&canonical_skills_dir(env))
}

/// Scan the disabled dir, collecting disabled skills by reported name.
pub fn scan_disabled(env: &Env) -> Vec<ScannedSkill> {
    scan_skills_in(&disabled_skills_dir(env))
}

/// Move a skill directory between the canonical dir and the disabled dir.
///
/// `to_enabled=true` moves `disabled-skills/<name>` → `skills/<name>` (enable);
/// `to_enabled=false` moves `skills/<name>` → `disabled-skills/<name>` (disable).
/// `name` is the **on-disk directory name** ([`ScannedSkill::dir_name`]) and
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
    fn sanitize_name_avoids_windows_reserved_device_names() {
        assert_eq!(sanitize_name("AUX"), "aux_");
        assert_eq!(sanitize_name("con"), "con_");
        assert_eq!(sanitize_name("aux.txt"), "aux.txt_");
        // Not reserved: unchanged.
        assert_eq!(sanitize_name("com10"), "com10");
        assert_eq!(sanitize_name("consult"), "consult");
    }

    #[test]
    fn install_slot_avoids_windows_reserved_names() {
        // A source dir named like a Windows device gets a deterministic slot
        // the filesystem accepts, on every platform.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("aux");
        let skill = write_skill(&src, "aux");

        install_skill(&skill, &env).unwrap();
        assert!(tmp.path().join(".agents/skills/aux_/SKILL.md").exists());
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
        assert!(path_contains(&base, &existing));

        // Not-yet-created target under an existing base: the base canonicalizes
        // to an absolute path while the target cannot — the naive fallback made
        // this comparison fail (absolute vs raw) and reject a valid name.
        assert!(path_contains(&base, &base.join("beta")));

        // Fully not-yet-created base and target still resolve to one root.
        assert!(path_contains(
            &tmp.path().join("a/b"),
            &tmp.path().join("a/b/c")
        ));

        // Traversal outside the base is still rejected.
        assert!(!path_contains(&base, &tmp.path().join("elsewhere")));
    }

    #[test]
    fn install_skill_writes_canonical_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");

        let r = install_skill(&skill, &env).unwrap();
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

        let r = install_skill(&skill, &env).unwrap();
        assert!(r.skipped);
        assert!(src.join("SKILL.md").exists());
    }

    #[test]
    fn install_skill_skips_when_already_installed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let canonical_base = tmp.path().join(".agents/skills");
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");

        assert!(install_skill(&skill, &env).is_ok());

        // A second install with changed content is skipped, leaving v1 intact.
        fs::write(src.join("SKILL.md"), "v2").unwrap();
        let r = install_skill(&write_skill(&src, "pdf"), &env).unwrap();
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
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");

        assert!(install_skill(&skill, &env).is_ok());
        move_skill("pdf", false, &env).unwrap();

        let r = install_skill(&skill, &env).unwrap();
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
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");

        // An unreadable file makes fs::copy fail mid-install.
        let bad = src.join("bad.txt");
        fs::write(&bad, "x").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();

        assert!(
            install_skill(&skill, &env).is_err(),
            "copy should have failed"
        );

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

        let src = tmp.path().join("PDF Master");
        let r = install_skill(&write_skill(&src, "PDF Master"), &env).unwrap();

        assert!(r.skipped);
        assert!(!tmp.path().join(".agents/skills/pdf-master").exists());
    }

    #[test]
    fn install_skill_keeps_non_ascii_names_on_separate_slots() {
        // Two Chinese-named skills from one source: the second must land on its own
        // slot instead of being reported as a copy of the first.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let first = write_skill(&tmp.path().join("中文技能"), "中文技能");
        let second = write_skill(&tmp.path().join("另一技能"), "另一技能");

        install_skill(&first, &env).unwrap();
        let r = install_skill(&second, &env).unwrap();
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
    fn scan_installed_finds_canonical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env).unwrap();

        let installed = scan_installed(&env);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "pdf");
        assert_eq!(installed[0].path, tmp.path().join(".agents/skills/pdf"));
    }

    #[test]
    fn scan_reports_the_frontmatter_name_and_the_real_path() {
        // An adopted skill keeps its original directory name, which can differ
        // from the frontmatter `name`: the scan reports the slugified name and
        // carries the real directory plus the raw name for display.
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let dir = tmp.path().join(".agents/skills/PDF Master");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: pdf-master\ndescription: d\n---\nbody",
        )
        .unwrap();

        let installed = scan_installed(&env);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "pdf-master");
        assert_eq!(installed[0].dir_name, "PDF Master");
        assert_eq!(installed[0].path, dir);

        // A manifest without a non-empty `name` is not a skill: the directory
        // is never scanned.
        let dir = tmp.path().join(".agents/skills/acrobat");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        fs::create_dir_all(tmp.path().join(".agents/skills/empty")).unwrap();
        let installed = scan_installed(&env);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "pdf-master");
    }

    #[test]
    fn move_skill_disables_then_enables_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env).unwrap();

        // Disable: moves out of canonical, into disabled-skills.
        move_skill("pdf", false, &env).unwrap();
        assert!(!tmp.path().join(".agents/skills/pdf").exists());
        assert!(
            tmp.path()
                .join(".agents/disabled-skills/pdf/SKILL.md")
                .exists()
        );
        assert!(scan_installed(&env).is_empty());
        let disabled = scan_disabled(&env);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].name, "pdf");

        // Enable: moves back.
        move_skill("pdf", true, &env).unwrap();
        assert!(tmp.path().join(".agents/skills/pdf/SKILL.md").exists());
        assert!(scan_disabled(&env).is_empty());
    }

    #[test]
    fn scan_disabled_reports_hidden_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        let src = tmp.path().join("pdf");
        let skill = write_skill(&src, "pdf");
        install_skill(&skill, &env).unwrap();
        move_skill("pdf", false, &env).unwrap();

        let disabled = scan_disabled(&env);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].name, "pdf");
    }
}
