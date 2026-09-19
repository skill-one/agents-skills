//! CLI command tree definition (clap derive).
//!
//! This is the externally exposed interface contract: the subcommands +
//! all flags, centralized in this file for readability. No subcommand
//! aliases — the full names are short and unambiguous (cargo-style minimalism).

use clap::{ArgGroup, Args, Parser, Subcommand};

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
    /// Add a skill package
    Add(AddArgs),
    /// Remove installed skills
    Remove(RemoveArgs),
    /// List installed skills
    List(ListArgs),
    /// Disable installed skills
    Disable(DisableArgs),
    /// Enable previously disabled skills
    Enable(EnableArgs),
    /// Manage agents' skills dirs link state (--link / --unlink / --status)
    Agent(AgentArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Source(s) to install
    #[arg(required = true)]
    pub source: Vec<String>,
    /// Specify skill names to install (use '*' for all skills)
    #[arg(short = 's', long = "skill", num_args = 1..)]
    pub skill: Vec<String>,
    /// List available skills in the repository without installing
    #[arg(short = 'l', long = "list")]
    pub list: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Skill names to remove
    pub skills: Vec<String>,
    /// Specify skills to remove (use '*' for all skills)
    #[arg(short = 's', long = "skill", num_args = 1..)]
    pub skill: Vec<String>,
    /// Remove all installed skills
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
pub struct DisableArgs {
    /// Skill names to disable
    pub skills: Vec<String>,
    /// Specify skills to disable (use '*' for all skills)
    #[arg(short = 's', long = "skill", num_args = 1..)]
    pub skill: Vec<String>,
    /// Disable all currently enabled skills
    #[arg(long = "all")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct EnableArgs {
    /// Skill names to enable
    pub skills: Vec<String>,
    /// Specify skills to enable (use '*' for all skills)
    #[arg(short = 's', long = "skill", num_args = 1..)]
    pub skill: Vec<String>,
    /// Enable all currently disabled skills
    #[arg(long = "all")]
    pub all: bool,
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
// Banner.
// ============================================================================

pub const RESET: &str = "\x1b[0m";
/// 256-color grayscale, readable on both dark and light backgrounds.
pub const DIM: &str = "\x1b[38;5;102m";
pub const TEXT: &str = "\x1b[38;5;145m";
pub const BOLD: &str = "\x1b[1m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";

/// Banner printed when no args are given (experimental commands removed).
pub fn show_banner() {
    println!();
    println!("{DIM}Agents skills installer and manager{RESET}");
    println!();
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills add {DIM}<package>{RESET}        {DIM}Add a new skill{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills remove{RESET}               {DIM}Remove installed skills{RESET}"
    );
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills list{RESET}                 {DIM}List installed skills{RESET}"
    );
    println!();
    println!(
        "  {DIM}${RESET} {TEXT}agents-skills update{RESET}               {DIM}Update installed skills{RESET}"
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
    println!("{DIM}try:{RESET} agents-skills add anthropics/skills");
    println!();
}
