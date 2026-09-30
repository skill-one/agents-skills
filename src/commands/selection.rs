//! remove / disable / enable: select installed skills by name and apply an
//! action — delete it, park it in the disabled dir, or restore it. The three
//! commands share one request shape ([`SelectionRequest`]) and one renderer;
//! only the wording differs. No business logic lives here.

use crate::cli::{BOLD, CYAN, DIM, GREEN, RESET, SelectionArgs, YELLOW};
use crate::commands::fail_agents;
use agents_skills::error::Result;
use agents_skills::{Manager, SelectionOutcome, SelectionRequest};

/// One of the three selection subcommands.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    /// Delete installed skills (canonical dir + any parked copy).
    Remove,
    /// Park installed skills in the disabled dir.
    Disable,
    /// Restore disabled skills into the canonical dir.
    Enable,
}

impl Action {
    /// The manager method backing this action.
    fn apply(&self, manager: &Manager, req: &SelectionRequest) -> Result<SelectionOutcome> {
        match self {
            Action::Remove => manager.remove(req),
            Action::Disable => manager.disable(req),
            Action::Enable => manager.enable(req),
        }
    }

    /// Static wording of the action in the renderer.
    fn verb(&self) -> &'static Verb {
        match self {
            Action::Remove => &REMOVE,
            Action::Disable => &DISABLE,
            Action::Enable => &ENABLE,
        }
    }
}

/// Static wording of one selection command.
struct Verb {
    /// Subcommand name (usage line).
    command: &'static str,
    /// Past tense used as the applied-line prefix ("Removed pdf").
    past: &'static str,
    /// Past participle ("pdf already removed").
    participle: &'static str,
    /// Header for the no-args listing ("Installed skills:").
    available_header: &'static str,
    /// Message when there is nothing selectable.
    none_available: &'static str,
    /// Footnote printed when something was applied.
    note: &'static str,
}

const REMOVE: Verb = Verb {
    command: "remove",
    past: "Removed",
    participle: "removed",
    available_header: "Installed skills:",
    none_available: "No skills found to remove.",
    note: "Removed for every linked agent (they share the canonical skills dir).",
};

const DISABLE: Verb = Verb {
    command: "disable",
    past: "Disabled",
    participle: "disabled",
    available_header: "Enabled skills:",
    none_available: "No enabled skills found to disable.",
    note: "Hidden from every linked agent (they share the canonical skills dir).",
};

const ENABLE: Verb = Verb {
    command: "enable",
    past: "Enabled",
    participle: "enabled",
    available_header: "Disabled skills:",
    none_available: "No disabled skills found to enable.",
    note: "Visible to every linked agent (they share the canonical skills dir).",
};

/// Run one selection command; renders the [`SelectionOutcome`].
pub fn run(manager: &Manager, args: SelectionArgs, action: Action) -> Result<()> {
    let req = SelectionRequest {
        skills: args
            .skills
            .iter()
            .chain(args.skill.iter())
            .cloned()
            .collect(),
        all: args.all,
    };
    let outcome = match action.apply(manager, &req) {
        Ok(o) => o,
        Err(e) => return fail_agents(e),
    };
    render(&req, &outcome, action.verb());
    // Requested names that matched nothing are a failure for scripts, even
    // though the rest of the request may have applied.
    if !outcome.missing.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn render(req: &SelectionRequest, outcome: &SelectionOutcome, verb: &Verb) {
    // List-only mode (no skills and not --all).
    if req.skills.is_empty() && !req.all {
        if outcome.available.is_empty() {
            println!("{YELLOW}{}{RESET}", verb.none_available);
        } else {
            println!("{BOLD}{}{RESET}", verb.available_header);
            for name in &outcome.available {
                println!("  {CYAN}{name}{RESET}");
            }
            println!();
            println!(
                "{DIM}Usage: agents-skills {} <name> [options]{RESET}",
                verb.command
            );
            println!("{DIM}Options: -s/--skill, --all{RESET}");
        }
        return;
    }

    // Nothing requested (e.g. --all with an empty set).
    if outcome.requested.is_empty() {
        println!("{YELLOW}{}{RESET}", verb.none_available);
        return;
    }

    for name in &outcome.applied {
        println!("{GREEN}✓{RESET} {} {name}", verb.past);
    }
    for name in &outcome.already {
        println!("{DIM}• {name} already {}{RESET}", verb.participle);
    }
    for name in &outcome.missing {
        println!("{YELLOW}! {name} not found{RESET}");
    }

    println!();
    if !outcome.applied.is_empty() {
        println!(
            "{GREEN}✓ {} {} skill(s){RESET}",
            verb.past,
            outcome.applied.len()
        );
        println!("{DIM}{}{RESET}", verb.note);
    }
    println!();
}
