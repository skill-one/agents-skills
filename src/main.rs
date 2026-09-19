//! agents-skills — a minimal, stable skill installer and manager for AI agents.
//!
//! Bin entry point: no args prints the banner; `-v/--version` prints a bare semver;
//! otherwise dispatch by subcommand. All business logic lives in the `agents-skills` library.

mod cli;
mod commands;

use agents_skills::Manager;
use agents_skills::error::Result;
use clap::Parser;
use cli::{BOLD, Cli, Command, RESET};

fn main() {
    // No args: print the banner.
    if std::env::args_os().len() == 1 {
        cli::show_banner();
        return;
    }

    // Handle clap parse errors uniformly: unknown subcommands exit 1; help-like info goes to stdout with exit 0, other errors exit 1.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if e.kind() == clap::error::ErrorKind::InvalidSubcommand {
                let cmd = std::env::args().nth(1).unwrap_or_default();
                println!("Unknown command: {cmd}");
                println!("Run {BOLD}agents-skills --help{RESET} for usage.");
                std::process::exit(1);
            }
            let code = if e.use_stderr() { 1 } else { 0 };
            let _ = e.print();
            std::process::exit(code);
        }
    };

    // Custom version: print the bare semver.
    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let manager = Manager::new();

    let result: Result<()> = match cli.command {
        Some(Command::Add(a)) => commands::add::run(&manager, a),
        Some(Command::Remove(r)) => commands::remove::run(&manager, r),
        Some(Command::List(l)) => commands::list::run(&manager, l),
        Some(Command::Disable(d)) => commands::disable::run(&manager, d),
        Some(Command::Enable(e)) => commands::enable::run(&manager, e),
        Some(Command::Agent(a)) => commands::agent::run(&manager, a),
        None => Ok(()),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
