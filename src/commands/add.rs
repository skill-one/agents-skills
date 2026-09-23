//! add: install one skill (local directory or `owner/repo@<skill>`).
//!
//! Renders the [`Manager::add`] outcome; no business logic lives here.

use crate::cli::{AddArgs, CYAN, DIM, GREEN, RESET, YELLOW};
use crate::commands::{fail_agents, shorten_path};
use agents_skills::error::Result;
use agents_skills::{AddOutcome, AddRequest, Env, Manager, SkillsError, SourceType};

pub fn run(manager: &Manager, args: AddArgs) -> Result<()> {
    let req = AddRequest {
        source: args.source,
        reference: args.reference,
    };

    let outcome = match manager.add(&req) {
        Ok(o) => o,
        Err(e) => return fail_add(e),
    };
    render(manager.env(), &req, &outcome);

    println!();
    println!(
        "{GREEN}Done!{RESET}{DIM}  Review skills before use; they run with full agent permissions.{RESET}"
    );
    Ok(())
}

fn fail_add(e: SkillsError) -> Result<()> {
    match e {
        SkillsError::Message(msg) => {
            println!("\x1b[31m{msg}\x1b[0m");
            std::process::exit(1);
        }
        other => fail_agents(other),
    }
}

fn render(env: &Env, req: &AddRequest, outcome: &AddOutcome) {
    print_source(env, req, &outcome.source);

    println!("Skill: {CYAN}{}{RESET}", outcome.skill.name);
    println!("{DIM}{}{RESET}", outcome.skill.description);

    println!();
    if outcome.skipped {
        println!(
            "{YELLOW}•{RESET} {} {DIM}skipped (already installed){RESET}",
            outcome.skill.name
        );
        println!("{DIM}To replace an installed skill: remove it first, then add again.{RESET}");
    } else {
        println!(
            "{GREEN}✓{RESET} {}",
            shorten_path(&outcome.canonical_path, env)
        );
        println!();
        println!("{GREEN}Installed 1 skill{RESET}");
    }
}

fn print_source(env: &Env, req: &AddRequest, parsed: &agents_skills::Source) {
    let main = match parsed.ty {
        SourceType::Local => parsed
            .local_path
            .as_ref()
            .map(|p| shorten_path(p, env))
            .unwrap_or_default(),
        SourceType::Github => format!("{}@{CYAN}{}{RESET}", parsed.slug(), parsed.skill),
    };
    let mut line = format!("Source: {main}");
    if let Some(r) = &req.reference {
        line.push_str(&format!(" {DIM}@ {YELLOW}{r}{RESET}"));
    }
    println!("{line}");
}
