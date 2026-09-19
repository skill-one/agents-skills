//! commands: the CLI command layer — arg unpacking + rendering.
//!
//! Business logic lives in the `agents-skills` library (`Manager`), never here.
//! Shared rendering helpers (path shortening, link result lines) live in this module.

pub mod add;
pub mod agent;
pub mod disable;
pub mod enable;
pub mod list;
pub mod remove;

use std::path::Path;

use crate::cli::{DIM, GREEN, RED, RESET, YELLOW};
use agents_skills::error::Result;
use agents_skills::{AgentLinkResult, Env, LinkOutcome, SkillsError};

/// Render an invalid-agents error to stdout and exit 1 (a CLI-only concern).
pub fn fail_agents(e: SkillsError) -> Result<()> {
    match e {
        SkillsError::InvalidAgents(names) => {
            println!("{YELLOW}Invalid agents: {names}{RESET}");
            println!(
                "{DIM}Valid agents: {}{RESET}",
                agents_skills::agent_names().join(", ")
            );
            std::process::exit(1);
        }
        other => Err(other),
    }
}

/// Render one agent link/unlink result line (used by `link`).
pub fn render_link_result(r: &AgentLinkResult) {
    match &r.outcome {
        LinkOutcome::Linked {
            adopted,
            quarantined,
            conflicts,
        } => {
            println!("{GREEN}✓{RESET} {} linked", r.display);
            if !adopted.is_empty() {
                println!(
                    "  {DIM}adopted into the canonical dir: {}{RESET}",
                    adopted.join(", ")
                );
            }
            if !quarantined.is_empty() {
                println!(
                    "  {DIM}moved non-skill files into .misc/: {}{RESET}",
                    quarantined.join(", ")
                );
            }
            if !conflicts.is_empty() {
                println!(
                    "  {DIM}dropped (already in the canonical dir): {}{RESET}",
                    conflicts.join(", ")
                );
            }
        }
        LinkOutcome::AlreadyLinked => {
            println!("{DIM}• {} already linked (canonical dir){RESET}", r.display)
        }
        LinkOutcome::Refused { reason } => {
            println!("{YELLOW}!{RESET} {} {reason}", r.display);
        }
        LinkOutcome::Skipped => println!("{DIM}– {} skipped (not installed){RESET}", r.display),
        LinkOutcome::Unlinked => println!("{GREEN}✓{RESET} {} unlinked", r.display),
        LinkOutcome::NotLinked => {
            println!("{DIM}• {} not linked (nothing to do){RESET}", r.display)
        }
        LinkOutcome::Failed { error } => println!("{RED}✗{RESET} {}: {error}", r.display),
    }
}

/// Shorten a path for display: `~` for home, `.` for the process cwd prefix.
pub fn shorten_path(path: &Path, env: &Env) -> String {
    let full = path.to_string_lossy();
    let home_s = env.home.to_string_lossy();
    if full == home_s {
        return "~".to_string();
    }
    if let Some(rest) = full.strip_prefix(&*home_s)
        && (rest.starts_with('/') || rest.starts_with('\\'))
    {
        return format!("~{rest}");
    }
    // Compare against the process cwd, not `env.cwd`: with `--project <dir>` the
    // manager targets another directory, and `./`-prefixing it would mislead.
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_s = cwd.to_string_lossy();
        if full == cwd_s {
            return ".".to_string();
        }
        if let Some(rest) = full.strip_prefix(&*cwd_s)
            && (rest.starts_with('/') || rest.starts_with('\\'))
        {
            return format!(".{rest}");
        }
    }
    full.to_string()
}
