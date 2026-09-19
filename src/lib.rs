//! `agents-skills` as a library: the high-level [`Manager`] facade over a private
//! domain layer.
//!
//! The library is pure data — it never prints to stdout/stderr and never calls
//! `process::exit`. The CLI binary (see `src/main.rs` + `src/commands`) is responsible
//! for rendering outcomes and deciding exit codes.
//!
//! # Quick tour
//!
//! ```
//! use agents_skills::{ListRequest, Manager};
//!
//! // Point the manager at a scratch environment (hermetic, no real home access).
//! let manager = Manager::builder()
//!     .home("/tmp/home")
//!     .config("/tmp/config")
//!     .cwd("/tmp/project")
//!     .build();
//!
//! // Install every skill from a source (see [`Manager::add`] for a local example).
//! // List what's installed.
//! let skills = manager.list(&ListRequest::default())?;
//!
//! # Ok::<(), agents_skills::Error>(())
//! ```
//!
//! # Layering
//!
//! The crate root exposes only the high-level [`Manager`] facade, its request/outcome
//! types, the few data types the outcomes carry ([`Env`], [`Source`], [`Skill`]), and
//! the unified [`error`] types. All domain logic (source parsing, agent directories,
//! SKILL.md discovery, install, agent links) lives in the private `core` module —
//! an implementation detail that may change without a breaking release.

#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod core;
pub mod error;
pub mod manager;

// High-level facade.
pub use manager::{
    AddOutcome, AddRequest, AgentLinkResult, AgentOutcome, AgentRequest, AgentStatus,
    DisableOutcome, DisableRequest, EnableOutcome, EnableRequest, InstallFailure, InstallSuccess,
    ListRequest, ListedSkill, Manager, ManagerBuilder, RemoveOutcome, RemoveRequest,
};

// Data types carried by the facade's outcomes (implementation lives in the private core).
pub use core::agents::Env;
pub use core::discover::Skill;
pub use core::link::LinkOutcome;
pub use core::source::{Source, SourceType};

// Errors.
pub use error::{Error, Result, SkillsError};

/// Every known agent identifier, in table order (useful for rendering choices).
pub fn agent_names() -> Vec<&'static str> {
    core::agents::AGENTS
        .iter()
        .map(|a| a.name.as_str())
        .collect()
}
