//! agent: manage agents' skills dirs and their link state relative to the canonical dir.
//!
//! Three modes, selected by a required flag — mirroring [`AgentRequest`] on the library side:
//! - `--link`: connect agents' skills dirs via directory-level symlinks (existing
//!   content is adopted into the canonical dir)
//! - `--status`: show which agents are linked ([`Manager::agent_status`])
//! - `--unlink`: disconnect agents' skills dirs (adopted content stays canonical)
//!
//! Renders outcomes; no business logic lives here.

use crate::cli::{BOLD, DIM, GREEN, RESET, YELLOW};
use crate::commands::{fail_agents, render_link_result};
use agents_skills::error::Result;
use agents_skills::{AgentOutcome, AgentRequest, Manager};

/// Run the `agent` command; `--status` reads only, `--unlink` disconnects, otherwise link.
pub fn run(manager: &Manager, args: crate::cli::AgentArgs) -> Result<()> {
    if args.status {
        render_status(manager);
        return Ok(());
    }
    let req = AgentRequest {
        agents: args.agents,
        unlink: args.unlink,
    };
    let outcome = match manager.agent(&req) {
        Ok(o) => o,
        Err(e) => return fail_agents(e),
    };
    render_link(&outcome, args.unlink);
    Ok(())
}

fn render_status(manager: &Manager) {
    println!("{BOLD}Agent link status{RESET}");
    println!();
    // Order comes from the library: canonical agents first, others keep table order.
    for s in manager.agent_status() {
        if s.canonical {
            println!(
                "  {DIM}•{RESET} {} {DIM}({}) — canonical{RESET}",
                s.display, s.name
            );
        } else if s.linked {
            println!(
                "  {GREEN}✓{RESET} {} {DIM}({}) — linked{RESET}",
                s.display, s.name
            );
        } else {
            println!(
                "  {YELLOW}!{RESET} {} {DIM}({}) — not linked{RESET}",
                s.display, s.name
            );
            if !s.internal_skills.is_empty() {
                println!(
                    "      {DIM}private skills: {}{RESET}",
                    s.internal_skills.join(", ")
                );
            }
            if !s.internal_others.is_empty() {
                println!(
                    "      {DIM}other files: {}{RESET}",
                    s.internal_others.join(", ")
                );
            }
        }
    }
    println!();
}

fn render_link(outcome: &AgentOutcome, unlink: bool) {
    if unlink {
        println!("{DIM}Unlinking agents from the canonical skills dir{RESET}");
    } else {
        println!("{DIM}Linking agents to the canonical skills dir{RESET}");
    }
    println!();
    for r in &outcome.results {
        render_link_result(r);
    }
    if unlink {
        println!();
        println!(
            "{DIM}Skills adopted into the canonical dir stay there; use `remove` to delete them.{RESET}"
        );
    }
    println!();
}
