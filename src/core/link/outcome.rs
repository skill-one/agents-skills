//! Outcome of linking/unlinking one agent's skills dir to the canonical dir.
//!
//! Splitting the enum into its own module keeps it reviewable: it is part of the
//! crate's public API while the rest of the link machinery is an implementation
//! detail.

/// Outcome of linking one agent's skills dir to the canonical dir.
#[derive(Debug)]
pub enum LinkOutcome {
    /// A new directory-level symlink was created. Pre-existing content (if any)
    /// was moved into the canonical dir and is not restored by unlink.
    Linked {
        /// Skill entries moved into the canonical dir.
        adopted: Vec<String>,
        /// Non-skill entries moved into `.misc/<agent>/` inside the canonical dir.
        quarantined: Vec<String>,
        /// Entries dropped because the canonical dir (or the `disabled-skills`
        /// dir) already holds that name — the existing copy wins.
        conflicts: Vec<String>,
    },
    /// The agent already uses the canonical dir (linked, or universal).
    AlreadyLinked,
    /// Linking was refused: the agent dir is a foreign symlink.
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
    /// The agent's skills dir was unlinked from the canonical dir. Skills moved
    /// into the canonical dir on link stay there (use `remove` to delete them).
    Unlinked,
    /// The agent's skills dir is not a link to the canonical dir (nothing to unlink).
    NotLinked,
}
