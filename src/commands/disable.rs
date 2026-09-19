//! disable: move installed skills into the disabled dir. Renders the [`Manager::disable`] outcome.

use crate::cli::{BOLD, CYAN, DIM, DisableArgs, GREEN, RESET, YELLOW};
use agents_skills::error::Result;
use agents_skills::{DisableOutcome, DisableRequest, Manager};

pub fn run(manager: &Manager, args: DisableArgs) -> Result<()> {
    let req = DisableRequest {
        skills: args
            .skills
            .iter()
            .chain(args.skill.iter())
            .cloned()
            .collect(),
        all: args.all,
    };
    let outcome = manager.disable(&req)?;
    render(&req, &outcome);
    Ok(())
}

fn render(req: &DisableRequest, outcome: &DisableOutcome) {
    // List-only mode (no skills and not --all).
    if req.skills.is_empty() && !req.all {
        if outcome.installed.is_empty() {
            println!("{YELLOW}No enabled skills found to disable.{RESET}");
        } else {
            println!("{BOLD}Enabled skills:{RESET}");
            for name in &outcome.installed {
                println!("  {CYAN}{name}{RESET}");
            }
            println!();
            println!("{DIM}Usage: agents-skills disable <name> [options]{RESET}");
            println!("{DIM}Options: -s/--skill, --all{RESET}");
        }
        return;
    }

    for name in &outcome.disabled {
        println!("{GREEN}✓{RESET} Disabled {name}");
    }
    for name in &outcome.already {
        println!("{DIM}• {name} already disabled{RESET}");
    }
    for name in &outcome.missing {
        println!("{YELLOW}! {name} not found{RESET}");
    }

    println!();
    if !outcome.disabled.is_empty() {
        println!(
            "{GREEN}✓ Disabled {} skill(s){RESET}",
            outcome.disabled.len()
        );
        println!(
            "{DIM}Hidden from every linked agent (they share the canonical skills dir).{RESET}"
        );
    }
    println!();
}
