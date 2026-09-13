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
pub fn render_link_result(r: &AgentLinkResult, env: &Env) {
    match &r.outcome {
        LinkOutcome::Linked {
            parked_skills,
            parked_others,
            backup_dir,
        } => {
            println!("{GREEN}✓{RESET} {} linked", r.display);
            render_parked(parked_skills, parked_others, backup_dir.as_deref(), env);
        }
        LinkOutcome::AlreadyLinked => {
            println!("{DIM}• {} already linked (canonical dir){RESET}", r.display)
        }
        LinkOutcome::Migrated {
            moved,
            skipped,
            parked_others,
            backup_dir,
        } => {
            let migrated = if moved.is_empty() {
                String::new()
            } else {
                format!(" {DIM}(migrated {}){RESET}", moved.join(", "))
            };
            println!("{GREEN}✓{RESET} {} linked{migrated}", r.display);
            if !skipped.is_empty() {
                println!(
                    "  {DIM}kept canonical copies for: {}{RESET}",
                    skipped.join(", ")
                );
            }
            render_parked(skipped, parked_others, backup_dir.as_deref(), env);
        }
        LinkOutcome::Refused { reason } => {
            println!("{YELLOW}!{RESET} {} {reason}", r.display);
        }
        LinkOutcome::Skipped => println!("{DIM}– {} skipped (not installed){RESET}", r.display),
        LinkOutcome::Unlinked {
            restored,
            restored_from,
        } => {
            println!("{GREEN}✓{RESET} {} unlinked", r.display);
            if let Some(from) = restored_from {
                if restored.is_empty() {
                    println!(
                        "  {DIM}restored nothing (backup at {} was empty){RESET}",
                        shorten_path(from, env)
                    );
                } else {
                    println!(
                        "  {DIM}restored {} from {}{RESET}",
                        restored.join(", "),
                        shorten_path(from, env)
                    );
                }
            }
        }
        LinkOutcome::NotLinked => {
            println!("{DIM}• {} not linked (nothing to do){RESET}", r.display)
        }
        LinkOutcome::Failed { error } => println!("{RED}✗{RESET} {}: {error}", r.display),
    }
}

/// Render the backup-slot note under a link line: what was parked and where.
fn render_parked(skills: &[String], others: &[String], dir: Option<&Path>, env: &Env) {
    let Some(dir) = dir else {
        return;
    };
    let mut names: Vec<&str> = skills.iter().map(String::as_str).collect();
    names.extend(others.iter().map(String::as_str));
    if names.is_empty() {
        return;
    }
    println!(
        "  {DIM}parked existing content at {} ({}) — unlink restores it{RESET}",
        shorten_path(dir, env),
        names.join(", ")
    );
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
