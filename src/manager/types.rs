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
/// The struct is `Default + Clone`; use [`AddRequest::new`] for the common
/// "install everything from a source" case and struct-update syntax
/// (`..Default::default()`) to override just the fields you need.
///
/// See the crate-level [source formats](crate#source-formats) table for the accepted
/// `source` strings.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    /// Source string (local path, GitHub `owner/repo`, git URL, or download URL).
    pub source: String,
    /// `"*"` or specific skill names; empty = all discovered skills.
    pub skills: Vec<String>,
    /// List available skills without installing anything.
    pub list_only: bool,
}

impl AddRequest {
    /// Create a request that installs all skills from `source` with default options.
    ///
    /// All other fields default: project scope, all skills.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{AddRequest, Manager};
    ///
    /// let req = AddRequest::new("anthropics/skills");
    /// assert_eq!(req.source, "anthropics/skills");
    /// assert!(req.skills.is_empty()); // all discovered skills
    /// # let _ = Manager::new();
    /// ```
    pub fn new(source: impl Into<String>) -> Self {
        AddRequest {
            source: source.into(),
            ..Default::default()
        }
    }
}

/// Request for [`Manager::remove`].
///
/// `Default` is a no-op that only reports installed names — set `skills` or `all` to
/// actually remove anything.
#[derive(Debug, Clone, Default)]
pub struct RemoveRequest {
    /// Skill names to remove (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Remove all installed skills.
    pub all: bool,
}

/// Request for [`Manager::agent`] — one entry point mirroring the `agent` CLI command.
///
/// `Default` links the auto-detected installed agents at project scope.
#[derive(Debug, Clone, Default)]
pub struct AgentRequest {
    /// `"*"` or specific agent names; empty = auto-detect installed agents.
    pub agents: Vec<String>,
    /// Unlink (disconnect) the agents' skills dirs instead of linking them.
    pub unlink: bool,
}

/// Request for [`Manager::disable`].
///
/// `Default` is a no-op that only reports enabled names — set `skills` or `all` to
/// actually disable anything.
#[derive(Debug, Clone, Default)]
pub struct DisableRequest {
    /// Skill names to disable (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Disable all currently enabled skills.
    pub all: bool,
}

/// Request for [`Manager::enable`].
///
/// `Default` is a no-op that only reports disabled names — set `skills` or `all` to
/// actually enable anything.
#[derive(Debug, Clone, Default)]
pub struct EnableRequest {
    /// Skill names to enable (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Enable all currently disabled skills.
    pub all: bool,
}

// ============================ Outcome types ============================

/// Result of [`Manager::add`].
///
/// Carries the full picture of an add operation: what was discovered, what was
/// selected, and which skills were installed into the canonical dir.
#[derive(Debug)]
pub struct AddOutcome {
    /// The parsed source.
    pub source: Source,
    /// All discovered skills.
    pub skills: Vec<Skill>,
    /// Selected skills (empty when `list_only`).
    pub selected: Vec<Skill>,
    /// Successfully installed skills.
    pub installed: Vec<InstallSuccess>,
    /// Selected skills left untouched because a skill of the same name is
    /// already installed (enabled or disabled) — `add` never overwrites.
    pub skipped: Vec<String>,
    /// Failed installations.
    pub failed: Vec<InstallFailure>,
    /// Whether this was a `--list` request.
    pub list_only: bool,
}

/// A single successful install (one skill, into the canonical dir).
#[derive(Debug)]
pub struct InstallSuccess {
    /// Skill name.
    pub name: String,
    /// Canonical directory.
    pub canonical_path: PathBuf,
}

/// A single failed install.
#[derive(Debug)]
pub struct InstallFailure {
    /// Skill name.
    pub skill: String,
    /// Error message.
    pub error: String,
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
    /// Skill name (from `SKILL.md` frontmatter).
    pub name: String,
    /// Skill description (from `SKILL.md` frontmatter).
    pub description: String,
    /// Directory the skill currently lives in (canonical, or `disabled-skills`).
    pub path: PathBuf,
    /// Whether the skill is enabled (`true`) or parked in `disabled-skills` (`false`).
    pub enabled: bool,
    /// The skill directory's creation time, as Unix seconds (UTC) — an
    /// approximation of when it landed on disk. Exact for `add` installs, but a
    /// skill adopted from an agent dir keeps that dir's original time. `None`
    /// when the platform/filesystem records no creation time (some Linux
    /// filesystems).
    pub installed_at: Option<u64>,
}

/// Result of [`Manager::remove`].
#[derive(Debug)]
pub struct RemoveOutcome {
    /// Installed names scanned (used by the no-args hint).
    pub installed: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually removed.
    pub removed: Vec<String>,
}

/// Result of [`Manager::disable`].
#[derive(Debug)]
pub struct DisableOutcome {
    /// Currently enabled names (used by the no-args hint).
    pub installed: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually disabled.
    pub disabled: Vec<String>,
    /// Names that were already disabled (idempotent no-op).
    pub already: Vec<String>,
    /// Requested names that matched neither enabled nor disabled skills.
    pub missing: Vec<String>,
}

/// Result of [`Manager::enable`].
#[derive(Debug)]
pub struct EnableOutcome {
    /// Currently disabled names (used by the no-args hint).
    pub disabled: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually enabled.
    pub enabled: Vec<String>,
    /// Names that were already enabled (idempotent no-op).
    pub already: Vec<String>,
    /// Requested names that matched neither enabled nor disabled skills.
    pub missing: Vec<String>,
}
