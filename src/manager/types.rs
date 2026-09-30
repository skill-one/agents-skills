//! Request and outcome data types carried by the `Manager` facade.
//!
//! Pure data (constructors and defaults only); kept in their own module so the
//! manager implementation in `mod.rs` stays focused on the methods.

use std::path::PathBuf;

use serde::Serialize;

use crate::core::discover::Skill;
use crate::core::link::LinkOutcome;
use crate::core::source::Source;

// ============================ Request types (clap-free) ============================

/// Request for [`Manager::add`].
///
/// The struct is `Default + Clone`; use [`AddRequest::new`] for the common case
/// and struct-update syntax (`..Default::default()`) to override just the fields
/// you need.
///
/// `source` accepts exactly two forms: a local skill directory (its `SKILL.md`
/// must declare a non-empty `name`), or the GitHub id `owner/repo/slug`
/// for one named skill on GitHub.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    /// Local skill directory, or the GitHub id `owner/repo/slug`.
    pub source: String,
    /// Branch, tag, or full commit SHA to pin (GitHub sources only; `None` = default branch).
    pub reference: Option<String>,
}

impl AddRequest {
    /// Create a request that installs the skill named by `source` with default options.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{AddRequest, Manager};
    ///
    /// let req = AddRequest::new("anthropics/skills/pdf");
    /// assert_eq!(req.source, "anthropics/skills/pdf");
    /// assert!(req.reference.is_none()); // default branch
    /// # let _ = Manager::new().ok();
    /// ```
    pub fn new(source: impl Into<String>) -> Self {
        AddRequest {
            source: source.into(),
            ..Default::default()
        }
    }
}

/// Request for [`Manager::remove`], [`Manager::disable`] and [`Manager::enable`]
/// — the three commands that select installed skills by name.
///
/// `Default` is a no-op that only reports the currently selectable names — set
/// `skills` or `all` to actually apply anything.
#[derive(Debug, Clone, Default)]
pub struct SelectionRequest {
    /// Skill names to select (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Select every currently available skill (all installed for
    /// remove/disable, all disabled for enable).
    pub all: bool,
}

/// Request for [`Manager::agent`] — one entry point mirroring the `agent` CLI command.
///
/// `Default` links the auto-detected installed agents.
#[derive(Debug, Clone, Default)]
pub struct AgentRequest {
    /// `"*"` or specific agent names; empty = auto-detect installed agents.
    pub agents: Vec<String>,
    /// Unlink (disconnect) the agents' skills dirs instead of linking them.
    pub unlink: bool,
}

// ============================ Outcome types ============================

/// Result of [`Manager::add`] — one source always resolves to exactly one skill.
#[derive(Debug)]
pub struct AddOutcome {
    /// The parsed source.
    pub source: Source,
    /// The skill that was targeted.
    pub skill: Skill,
    /// Canonical directory of the skill.
    pub canonical_path: PathBuf,
    /// `true` when nothing was copied because a skill of the same name is
    /// already installed (enabled or disabled) — `add` never overwrites.
    pub skipped: bool,
}

/// Result of linking (or unlinking) one agent's skills dir relative to the
/// canonical dir.
#[derive(Debug)]
pub struct AgentLinkResult {
    /// Agent identifier (as used on the CLI).
    pub agent: String,
    /// Agent display name.
    pub display: String,
    /// Link/unlink outcome details.
    pub outcome: LinkOutcome,
}

/// Link status of one agent (used by `agent --status`).
#[derive(Debug)]
pub struct AgentStatus {
    /// Agent identifier (as used on the CLI).
    pub name: String,
    /// Agent display name.
    pub display: String,
    /// Whether the agent's skills dir is linked to the canonical dir.
    pub linked: bool,
    /// Whether the agent natively uses the canonical dir (no link involved).
    pub canonical: bool,
    /// Skills inside the agent's own skills dir: real subdirs and dir-targeting
    /// symlinks — the same classification linking uses. Only populated for
    /// unlinked, non-canonical agents; empty for linked/canonical agents (they
    /// share the canonical dir, shown by `list`).
    pub internal_skills: Vec<String>,
    /// Non-skill entries (files, symlinks to non-directories) inside the agent's
    /// own skills dir. Same population rules as [`AgentStatus::internal_skills`].
    pub internal_others: Vec<String>,
}

/// Result of [`Manager::agent`].
#[derive(Debug)]
pub struct AgentOutcome {
    /// Per-agent link results.
    pub results: Vec<AgentLinkResult>,
}

/// A listed skill (serialized by `list --json`).
///
/// Fields are serialized in camelCase — the exact JSON shape emitted by the
/// CLI's `list --json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedSkill {
    /// Skill slug — the SKILL.md frontmatter `name` slugified (lowercase,
    /// spaces as dashes). This is the identity every command
    /// (`add`/`remove`/`disable`/`enable`) selects by, and the on-disk
    /// directory name of a fresh install. Directories whose manifest declares
    /// no `name` are not skills and are never listed.
    pub name: String,
    /// The frontmatter `name` as declared — a display-only rendition of the
    /// same skill (it may contain spaces and mixed case the slug folds away).
    pub display_name: String,
    /// Skill description (from `SKILL.md` frontmatter).
    pub description: String,
    /// The skill's real on-disk directory (in the canonical dir when enabled,
    /// in the sibling `disabled-skills` dir when disabled). The on-disk
    /// directory name can differ from the slug for skills adopted from an
    /// agent dir.
    pub path: std::path::PathBuf,
    /// Whether the skill is enabled (`true`) or parked in `disabled-skills` (`false`).
    pub enabled: bool,
    /// The skill directory's creation time, as Unix seconds (UTC) — an
    /// approximation of when it landed on disk. Exact for `add` installs, but a
    /// skill adopted from an agent dir keeps that dir's original time. `None`
    /// when the platform/filesystem records no creation time (some Linux
    /// filesystems).
    pub installed_at: Option<u64>,
}

/// Result of [`Manager::remove`], [`Manager::disable`] and [`Manager::enable`]
/// — the three selection commands share one outcome shape.
#[derive(Debug)]
pub struct SelectionOutcome {
    /// Names of the skills currently selectable by the command (installed
    /// skills for remove/disable, disabled skills for enable) — used by the
    /// no-args hint.
    pub available: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names the command actually applied to (removed / disabled / enabled),
    /// as reported.
    pub applied: Vec<String>,
    /// Names that were already in the target state (idempotent no-op).
    pub already: Vec<String>,
    /// Requested names that matched nothing.
    pub missing: Vec<String>,
}
