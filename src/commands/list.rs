//! list: list installed skills (project/global, `--json`, `-a` agent filter).

use crate::cli::{BOLD, CYAN, DIM, GREEN, ListArgs, RESET, YELLOW};
use crate::commands::{fail_agents, shorten_path};
use agents_skills::error::Result;
use agents_skills::{Env, ListRequest, ListedSkill, Manager};

pub fn run(manager: &Manager, args: ListArgs) -> Result<()> {
    let global = args.project.is_none();
    let req = ListRequest { global };
    let listed = match manager.list(&req) {
        Ok(l) => l,
        Err(e) => return fail_agents(e),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&listed)?);
        return Ok(());
    }

    let scope_label = if global { "Global" } else { "Project" };
    if listed.is_empty() {
        println!(
            "{DIM}No {} skills found.{RESET}",
            scope_label.to_lowercase()
        );
        if global {
            println!("{DIM}Try listing project skills with --project{RESET}");
        } else {
            println!("{DIM}Try listing global skills without --project{RESET}");
        }
        return Ok(());
    }

    println!("{BOLD}{} Skills{RESET}", scope_label);
    println!();
    for skill in &listed {
        print_skill(skill, manager.env());
    }
    println!();
    Ok(())
}

fn print_skill(skill: &ListedSkill, env: &Env) {
    let short = shorten_path(&skill.path, env);
    let status = if skill.enabled {
        format!("{GREEN}enabled{RESET}")
    } else {
        format!("{YELLOW}disabled{RESET}")
    };
    println!(
        "{CYAN}{}{RESET} {DIM}{}{RESET} [{status}]",
        skill.name, short
    );
}
