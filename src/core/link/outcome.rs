//! Outcome of linking/unlinking one agent's skills dir to the canonical dir.
//!
//! Splitting the enum into its own module keeps it reviewable: it is part of the
//! crate's public API while the rest of the link machinery is an implementation
//! detail.

use std::path::PathBuf;

/// Outcome of linking one agent's skills dir to the canonical dir.
#[derive(Debug)]
pub enum LinkOutcome {
    /// A new directory-level symlink was created. Pre-existing content (if any)
    /// was parked in the agent's backup slot; unlink restores it.
    Linked {
        /// Skill entries parked in the backup slot (reporting only).
        parked_skills: Vec<String>,
        /// Non-skill entries parked in the backup slot (reporting only).
        parked_others: Vec<String>,
        /// The parked dir inside the backup slot (None when nothing was parked).
        backup_dir: Option<PathBuf>,
    },
    /// The agent already uses the canonical dir (linked, or universal).
    AlreadyLinked,
    /// The skills dir was parked whole, then its skill dirs were moved into the
    /// canonical dir (`migrate`); everything else (name-clash copies, non-skill
    /// entries, old-model links) stays parked.
    Migrated {
        /// Names of the skill directories moved into the canonical dir.
        moved: Vec<String>,
        /// Skills whose name already exists in the canonical dir (the canonical
        /// copy wins) or is disabled in the `disabled-skills` dir (disabled
        /// wins); the agent-side copy stays parked in the backup slot.
        skipped: Vec<String>,
        /// Non-skill entries parked in the backup slot (reporting only).
        parked_others: Vec<String>,
        /// The parked dir inside the backup slot (None when nothing stays parked).
        backup_dir: Option<PathBuf>,
    },
    /// Linking was refused: the agent dir is a foreign symlink, or a previous
    /// backup is still parked.
    Refused {
        /// Human-readable reason and remedy.
        reason: String,
    },
    /// The agent is not present in this scope (its root dir does not exist).
    Skipped,
    /// The link could not be established.
    Failed {
        /// Error message.
        error: String,
    },
    /// The agent's skills dir was unlinked from the canonical dir; the parked
    /// dir (if any) was restored with a single rename into its place.
    Unlinked {
        /// Names restored from the backup slot (empty = nothing was parked).
        restored: Vec<String>,
        /// The parked dir the content came from (None when nothing was parked).
        restored_from: Option<PathBuf>,
    },
    /// The agent's skills dir is not a link to the canonical dir (nothing to unlink).
    NotLinked,
}
