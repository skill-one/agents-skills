//! remove: delete installed skills. Renders the [`Manager::remove`] outcome.

use crate::cli::{BOLD, CYAN, DIM, GREEN, RED, RESET, RemoveArgs, YELLOW};
use crate::commands::fail_agents;
use agents_skills::error::Result;
use agents_skills::{Manager, RemoveOutcome, RemoveRequest};

pub fn run(manager: &Manager, args: RemoveArgs) -> Result<()> {
    let req = RemoveRequest {
        skills: args
            .skills
            .iter()
            .chain(args.skill.iter())
            .cloned()
            .collect(),
        all: args.all,
    };
    let outcome = match manager.remove(&req) {
        Ok(o) => o,
        Err(e) => return fail_agents(e),
    };
    render(&req, &outcome);
    Ok(())
}

fn render(req: &RemoveRequest, outcome: &RemoveOutcome) {
    // List-only mode (no skills and not --all).
    if req.skills.is_empty() && !req.all {
        if outcome.installed.is_empty() {
            println!("{YELLOW}No skills found to remove.{RESET}");
        } else {
            println!("{BOLD}Installed skills:{RESET}");
            for name in &outcome.installed {
                println!("  {CYAN}{name}{RESET}");
            }
            println!();
            println!("{DIM}Usage: agents-skills remove <name> [options]{RESET}");
            println!("{DIM}Options: -s/--skill, --all{RESET}");
        }
        return;
    }

    // Nothing requested.
    if outcome.requested.is_empty() {
        println!("{YELLOW}No skills found to remove.{RESET}");
        return;
    }

    // No match.
    if outcome.removed.is_empty() {
        println!(
            "{RED}No matching skills found for: {}{RESET}",
            outcome.requested.join(", ")
        );
        return;
    }

    println!(
        "{GREEN}Successfully removed {} skill(s){RESET}",
        outcome.removed.len()
    );
    println!("{DIM}Removed for every linked agent (they share the canonical skills dir).{RESET}");
    println!();
    println!("{GREEN}Done!{RESET}");
}
