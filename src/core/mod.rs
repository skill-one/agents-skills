//! core: CLI-agnostic domain logic (source parsing, agent dirs, SKILL.md discovery, install, links).
//!
//! Everything is pure functions or dependency-injectable, for easy unit testing and reuse.

pub mod agents;
pub mod discover;
pub mod fetch;
pub mod github;
pub mod install;
pub mod link;
pub mod source;
pub mod tokens;

#[cfg(test)]
pub mod test_utils;
