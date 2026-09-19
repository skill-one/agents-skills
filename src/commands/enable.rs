//! enable: move disabled skills back into the canonical dir. Renders the [`Manager::enable`] outcome.

use crate::cli::{BOLD, CYAN, DIM, EnableArgs, GREEN, RESET, YELLOW};
use agents_skills::error::Result;
use agents_skills::{EnableOutcome, EnableRequest, Manager};

pub fn run(manager: &Manager, args: EnableArgs) -> Result<()> {
    let req = EnableRequest {
        skills: args
            .skills
            .iter()
            .chain(args.skill.iter())
            .cloned()
            .collect(),
        all: args.all,
    };
    let outcome = manager.enable(&req)?;
    render(&req, &outcome);
    Ok(())
}

fn render(req: &EnableRequest, outcome: &EnableOutcome) {
    // List-only mode (no skills and not --all).
    if req.skills.is_empty() && !req.all {
        if outcome.disabled.is_empty() {
            println!("{YELLOW}No disabled skills found to enable.{RESET}");
        } else {
            println!("{BOLD}Disabled skills:{RESET}");
            for name in &outcome.disabled {
                println!("  {CYAN}{name}{RESET}");
            }
            println!();
            println!("{DIM}Usage: agents-skills enable <name> [options]{RESET}");
            println!("{DIM}Options: -s/--skill, --all{RESET}");
        }
        return;
    }

    for name in &outcome.enabled {
        println!("{GREEN}✓{RESET} Enabled {name}");
    }
    for name in &outcome.already {
        println!("{DIM}• {name} already enabled{RESET}");
    }
    for name in &outcome.missing {
        println!("{YELLOW}! {name} not found{RESET}");
    }

    println!();
    if !outcome.enabled.is_empty() {
        println!("{GREEN}✓ Enabled {} skill(s){RESET}", outcome.enabled.len());
        println!(
            "{DIM}Visible to every linked agent (they share the canonical skills dir).{RESET}"
        );
    }
    println!();
}
