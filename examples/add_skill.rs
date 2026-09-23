//! Install a skill using the high-level [`Manager`] facade.
//!
//! Run with:
//!   cargo run --example add_skill -- anthropics/skills@pdf
//!   cargo run --example add_skill -- ./path/to/skill
//!
//! Note: this installs into your real environment. Run `agents-skills agent --link`
//! afterwards to expose the skill to your agents.

use agents_skills::{AddRequest, Manager};

fn main() -> agents_skills::Result<()> {
    let source = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "anthropics/skills@pdf".to_string());

    let manager = Manager::new();
    let outcome = manager.add(&AddRequest::new(source))?;

    if outcome.skipped {
        println!("Skipped (already installed): {}", outcome.skill.name);
    } else {
        println!("Installed: {}", outcome.skill.name);
    }
    Ok(())
}
