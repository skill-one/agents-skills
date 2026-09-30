//! CLI command tree definition (clap derive).
//!
//! This is the externally exposed interface contract: the subcommands +
//! all flags, centralized in this file for readability. No subcommand
//! aliases — the full names are short and unambiguous (cargo-style minimalism).

use clap::{ArgGroup, Args, Parser, Subcommand};
use std::io::IsTerminal;

#[derive(Debug, Parser)]
#[command(
    name = "agents-skills",
    about = "A minimal skill installer and manager for AI agents",
    long_about = None,
    disable_version_flag = true
)]
pub struct Cli {
    /// Show version number
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a skill (local directory or owner/repo/slug)
    Add(AddArgs),
    /// Remove installed skills
    Remove(SelectionArgs),
    /// List installed skills
    List(ListArgs),
    /// Disable installed skills
    Disable(SelectionArgs),
    /// Enable previously disabled skills
    Enable(SelectionArgs),
    /// Manage agents' skills dirs link state (--link / --unlink / --status)
    Agent(AgentArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// A local skill directory, or the GitHub id `owner/repo/slug`
    #[arg(required = true)]
    pub source: String,
    /// Pin a branch, tag, or full commit SHA (GitHub sources only)
    #[arg(long = "ref", value_name = "ref")]
    pub reference: Option<String>,
}

#[derive(Debug, Args)]
pub struct SelectionArgs {
    /// Skill names
    pub skills: Vec<String>,
    /// Skill name (repeatable)
    #[arg(short = 's', long = "skill", num_args = 1..)]
    pub skill: Vec<String>,
    /// All available skills (installed for remove/disable, disabled for enable)
    #[arg(long = "all")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Output as JSON (machine-readable, no ANSI codes)
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .args(["link", "unlink", "status"])
))]
pub struct AgentArgs {
    /// Agents to link/unlink (default: auto-detect installed agents; use '*' for all)
    pub agents: Vec<String>,
    /// Link agents' skills dirs to the canonical dir
    #[arg(long = "link")]
    pub link: bool,
    /// Unlink agents' skills dirs from the canonical dir
    #[arg(long = "unlink")]
    pub unlink: bool,
    /// Show link status of installed agents (does not modify anything)
    #[arg(long = "status")]
    pub status: bool,
}

// ============================================================================
// Banner + ANSI styles.
// ============================================================================

/// ANSI styles, rendered empty when colors are off: stdout is not a terminal
/// (piped, redirected) or `NO_COLOR` is set. Decided lazily, once per process.
pub struct Style(&'static str);

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn enabled() -> bool {
            use std::sync::OnceLock;
            static ENABLED: OnceLock<bool> = OnceLock::new();
            *ENABLED.get_or_init(|| {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            })
        }
        if enabled() {
            f.write_str(self.0)
        } else {
            Ok(())
        }
    }
}

pub static RESET: Style = Style("\x1b[0m");
/// 256-color grayscale, readable on both dark and light backgrounds.
pub static DIM: Style = Style("\x1b[38;5;102m");
pub static TEXT: Style = Style("\x1b[38;5;145m");
pub static BOLD: Style = Style("\x1b[1m");
pub static CYAN: Style = Style("\x1b[36m");
pub static GREEN: Style = Style("\x1b[32m");
pub static YELLOW: Style = Style("\x1b[33m");
pub static RED: Style = Style("\x1b[31m");

/// Banner printed when no args are given (experimental commands removed).
pub fn show_banner() {
    println!();
    println!("{DIM}Agents skills installer and manager{RESET}");
    println!();
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills add {DIM}owner/repo/slug{RESET}  {DIM}Add a skill{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills remove{RESET}               {DIM}Remove installed skills{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills list{RESET}                 {DIM}List installed skills{RESET}"
    );
    println!();
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills agent --link{RESET}          {DIM}Link agents to the skills dir{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills agent --status{RESET}        {DIM}Show agent link status{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills agent --unlink{RESET}        {DIM}Unlink agents{RESET}"
    );
    println!();
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills disable{RESET}            {DIM}Disable installed skills{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills enable{RESET}             {DIM}Re-enable disabled skills{RESET}"
    );
    println!();
    println!("{DIM}try:{RESET} agents-skills add anthropics/skills/pdf");
    println!();
}
