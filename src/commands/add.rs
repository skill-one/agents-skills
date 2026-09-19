//! add: install skills (local path / GitHub / git / download).
//!
//! Renders the [`Manager::add`] outcome; no business logic lives here.

use std::collections::HashSet;

use crate::cli::{AddArgs, BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW};
use crate::commands::{fail_agents, shorten_path};
use agents_skills::error::Result;
use agents_skills::{AddOutcome, AddRequest, Env, Manager, SkillsError, Source, SourceType};

pub fn run(manager: &Manager, args: AddArgs) -> Result<()> {
    for source in &args.source {
        let req = AddRequest {
            source: source.clone(),
            skills: args.skill.clone(),
            list_only: args.list,
        };

        let outcome = match manager.add(&req) {
            Ok(o) => o,
            Err(e) => return fail_add(e),
        };
        render(manager.env(), &req, &outcome);
    }

    println!();
    println!(
        "{GREEN}Done!{RESET}{DIM}  Review skills before use; they run with full agent permissions.{RESET}"
    );
    Ok(())
}

fn fail_add(e: SkillsError) -> Result<()> {
    match e {
        SkillsError::Message(msg) => {
            println!("{RED}{msg}{RESET}");
            std::process::exit(1);
        }
        other => fail_agents(other),
    }
}

fn render(env: &Env, req: &AddRequest, outcome: &AddOutcome) {
    print_source(&outcome.source);
    println!(
        "Found {GREEN}{}{RESET} skill{}",
        outcome.skills.len(),
        if outcome.skills.len() > 1 { "s" } else { "" }
    );

    if outcome.list_only {
        println!();
        println!("{BOLD}Available Skills{RESET}");
        for skill in &outcome.skills {
            println!("  {CYAN}{}{RESET}", skill.name);
            println!("    {DIM}{}{RESET}", skill.description);
        }
        println!();
        println!("Use --skill <name> to install specific skills");
        return;
    }

    // Selection message.
    let at_filter = outcome.source.skill_filter.clone();
    if req.skills.iter().any(|s| s == "*") {
        println!("Installing all {} skills", outcome.skills.len());
    } else if !req.skills.is_empty() || at_filter.is_some() {
        if outcome.selected.is_empty() {
            let names = if !req.skills.is_empty() {
                req.skills.join(", ")
            } else {
                at_filter.unwrap_or_default()
            };
            println!("{RED}No matching skills found for: {}{RESET}", names);
            println!("Available skills:");
            for s in &outcome.skills {
                println!("  - {}", s.name);
            }
            std::process::exit(1);
        }
        println!(
            "Selected {} skill{}: {}",
            outcome.selected.len(),
            if outcome.selected.len() != 1 { "s" } else { "" },
            outcome
                .selected
                .iter()
                .map(|s| CYAN.to_string() + &s.name + RESET)
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else if outcome.skills.len() == 1 {
        let first = &outcome.skills[0];
        println!("Skill: {CYAN}{}{RESET}", first.name);
        println!("{DIM}{}{RESET}", first.description);
    } else {
        println!("Installing all {} skills", outcome.skills.len());
    }

    // Results.
    if !outcome.installed.is_empty() {
        let count = outcome
            .installed
            .iter()
            .map(|i| i.name.as_str())
            .collect::<HashSet<_>>()
            .len();
        println!();
        for s in &outcome.installed {
            println!("{GREEN}✓{RESET} {}", shorten_path(&s.canonical_path, env));
        }
        println!();
        println!(
            "{GREEN}Installed {count} skill{}{RESET}",
            if count != 1 { "s" } else { "" }
        );
    }
    if !outcome.skipped.is_empty() {
        println!();
        for name in &outcome.skipped {
            println!("{YELLOW}•{RESET} {name} {DIM}skipped (already installed){RESET}");
        }
        println!("{DIM}To replace an installed skill: remove it first, then add again.{RESET}");
    }
    if !outcome.failed.is_empty() {
        println!();
        println!("{RED}Failed to install {}{RESET}", outcome.failed.len());
        for f in &outcome.failed {
            println!("  {RED}✗{RESET} {}: {DIM}{}{RESET}", f.skill, f.error);
        }
    }
    println!();
}

fn print_source(parsed: &Source) {
    let main = match parsed.ty {
        SourceType::Local => parsed
            .local_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        _ => parsed.url.clone(),
    };
    let mut line = format!("Source: {main}");
    if let Some(r) = &parsed.r#ref {
        line.push_str(&format!(" @ {YELLOW}{r}{RESET}"));
    }
    if let Some(sp) = &parsed.subpath {
        line.push_str(&format!(" ({sp})"));
    }
    if let Some(sf) = &parsed.skill_filter {
        line.push_str(&format!(" {DIM}@{RESET}{CYAN}{sf}{RESET}"));
    }
    println!("{line}");
}
