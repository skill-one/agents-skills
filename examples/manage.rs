//! A self-contained walkthrough of the `Manager` lifecycle (add → list → remove)
//! using an injected scratch directory, so it never touches your real home.
//!
//! Run with:
//!   cargo run --example manage

use std::path::Path;

use agents_skills::{AddRequest, Manager, RemoveRequest};

/// Create a minimal skill directory on disk for the demo.
fn write_skill(root: &Path, name: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: does {name}\n---\n\n# {name}\n"),
    )
    .unwrap();
    dir
}

fn main() -> agents_skills::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project)?;

    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .config(tmp.path().join("config"))
        .cwd(project)
        .build();

    // Add a local skill (installs into the canonical dir; no agent linking).
    let src = write_skill(tmp.path(), "hello");
    let outcome = manager.add(&AddRequest {
        source: src.display().to_string(),
        ..Default::default()
    })?;
    println!("Installed {} skill(s)", outcome.installed.len());

    // List installed skills (serde-serializable; same shape as `list --json`).
    let skills = manager.list()?;
    for s in &skills {
        println!("  - {}: {}", s.name, manager.skill_dir(s).display());
    }

    // Remove it again.
    let removed = manager.remove(&RemoveRequest {
        skills: vec!["hello".to_string()],
        ..Default::default()
    })?;
    println!("Removed: {:?}", removed.removed);

    Ok(())
}
