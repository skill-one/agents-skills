//! list: list installed skills (project/global, `--json`).

use crate::cli::{BOLD, CYAN, DIM, GREEN, ListArgs, RESET, YELLOW};
use crate::commands::{fail_agents, shorten_path};
use agents_skills::error::Result;
use agents_skills::{Env, ListedSkill, Manager};

/// Longest description rendered before it is ellipsized.
const DESCRIPTION_MAX: usize = 100;

pub fn run(manager: &Manager, args: ListArgs) -> Result<()> {
    let listed = match manager.list() {
        Ok(l) => l,
        Err(e) => return fail_agents(e),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&listed)?);
        return Ok(());
    }

    if listed.is_empty() {
        println!("{DIM}No skills installed.{RESET}");
        println!("{DIM}Try: agents-skills add <source>{RESET}");
        return Ok(());
    }

    println!("{BOLD}Skills{RESET}");
    println!();
    for skill in &listed {
        print_skill(skill, manager.env());
    }
    println!();
    Ok(())
}

fn print_skill(skill: &ListedSkill, env: &Env) {
    let status = if skill.enabled {
        format!("{GREEN}enabled{RESET}")
    } else {
        format!("{YELLOW}disabled{RESET}")
    };
    println!(
        "{CYAN}{}{RESET} {DIM}{}{RESET}",
        skill.name,
        truncate(&skill.description, DESCRIPTION_MAX)
    );
    let installed = match skill.installed_at.and_then(local_datetime) {
        Some(at) => format!(" {DIM}· {at}{RESET}"),
        None => String::new(),
    };
    println!(
        "  {DIM}{}{RESET} [{status}]{installed}",
        shorten_path(&skill.path, env)
    );
}

/// Truncate to `max` characters with an ellipsis. The library already collapses
/// the description onto a single line.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Format Unix seconds as a local `YYYY-MM-DD HH:MM` string.
fn local_datetime(secs: u64) -> Option<String> {
    let ts = jiff::Timestamp::from_second(secs as i64).ok()?;
    Some(
        ts.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
    )
}
