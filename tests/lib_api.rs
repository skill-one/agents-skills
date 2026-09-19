//! Integration tests for the library API: proving the lib works independently of the CLI,
//! driven entirely through `ManagerBuilder` + request structs (no `assert_cmd`).

use std::path::{Path, PathBuf};

use agents_skills::{
    AddRequest, AgentRequest, DisableRequest, EnableRequest, LinkOutcome, ListRequest, Manager,
    RemoveRequest, SkillsError,
};

fn write_skill_source(root: &Path, rel_dir: &str, name: &str) -> PathBuf {
    let dir = root.join(rel_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let md = format!("---\nname: {name}\ndescription: does {name}\n---\n\n# {name}\n");
    std::fs::write(dir.join("SKILL.md"), md).unwrap();
    dir
}

#[test]
fn lib_add_list_remove_roundtrip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();

    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    // Add a local skill.
    let src = write_skill_source(tmp.path(), "src", "pdf");
    let outcome = manager
        .add(&AddRequest {
            source: src.display().to_string(),
            ..Default::default()
        })
        .unwrap();

    assert_eq!(outcome.skills.len(), 1);
    assert_eq!(outcome.installed.len(), 1);
    assert!(outcome.failed.is_empty());
    assert!(cwd.join(".agents/skills/pdf/SKILL.md").exists());

    // List finds it.
    let listed = manager.list(&ListRequest::default()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "pdf");
    assert!(listed[0].enabled);

    // Remove it.
    let removed = manager
        .remove(&RemoveRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(removed.removed, vec!["pdf".to_string()]);
    assert!(!cwd.join(".agents/skills/pdf").exists());
}

#[test]
fn lib_add_global_uses_home() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    let src = write_skill_source(tmp.path(), "src", "pdf");
    let outcome = manager
        .add(&AddRequest {
            source: src.display().to_string(),
            global: true,
            ..Default::default()
        })
        .unwrap();

    assert_eq!(outcome.installed.len(), 1);
    assert!(home.join(".agents/skills/pdf/SKILL.md").exists());
    assert!(!cwd.join(".agents/skills/pdf").exists());
}

#[test]
fn lib_invalid_agent_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let manager = Manager::builder().cwd(tmp.path().join("project")).build();

    let err = manager
        .agent(&AgentRequest {
            agents: vec!["not-an-agent".to_string()],
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err, SkillsError::InvalidAgents(_)));
}

#[test]
fn lib_missing_local_path_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let manager = Manager::builder().cwd(tmp.path().join("project")).build();

    let err = manager
        .add(&AddRequest {
            source: tmp.path().join("nope").display().to_string(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(err.to_string().contains("Local path does not exist"));
}

#[test]
fn lib_list_json_shape() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();

    let manager = Manager::builder().cwd(cwd).build();
    let src = write_skill_source(tmp.path(), "src", "pdf");
    manager
        .add(&AddRequest {
            source: src.display().to_string(),
            ..Default::default()
        })
        .unwrap();

    let listed = manager.list(&ListRequest::default()).unwrap();
    let json = serde_json::to_string_pretty(&listed).unwrap();
    assert!(json.contains("\"name\": \"pdf\""));
    assert!(json.contains("\"enabled\": true"));
}

#[test]
fn lib_agent_status_reports_canonical_and_linked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    // cline is native at global scope too (global dir ~/.agents/skills).
    std::fs::create_dir_all(home.join(".cline")).unwrap();
    // codex is universal at PROJECT scope but reads ~/.codex/skills globally:
    // installed but not canonical until explicitly linked.
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    // Installed non-universal agent (trae detects ~/.trae) → linked after linking.
    std::fs::create_dir_all(home.join(".trae")).unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    manager
        .agent(&AgentRequest {
            agents: vec!["trae".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();

    let statuses = manager.agent_status(true);
    let trae = statuses.iter().find(|s| s.name == "trae").unwrap();
    assert!(!trae.canonical);
    assert!(trae.linked);
    let cline = statuses.iter().find(|s| s.name == "cline").unwrap();
    assert!(cline.canonical);
    assert!(cline.linked);
    // Scope-aware: codex is NOT canonical globally and starts unlinked.
    let codex = statuses.iter().find(|s| s.name == "codex").unwrap();
    assert!(!codex.canonical);
    assert!(!codex.linked);
    // Uninstalled agents (neither installed nor linked) are not reported.
    assert!(statuses.iter().all(|s| s.name != "claude-code"));
    // Uninstalled universal agents are not reported either.
    assert!(statuses.iter().all(|s| s.name != "amp"));

    // Linking codex at global scope connects its vendor dir.
    manager
        .agent(&AgentRequest {
            agents: vec!["codex".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();
    assert!(home.join(".codex/skills").is_symlink());
    let statuses = manager.agent_status(true);
    let codex = statuses.iter().find(|s| s.name == "codex").unwrap();
    assert!(!codex.canonical);
    assert!(codex.linked);
}

#[test]
fn lib_agent_status_reports_internal_skills_for_unlinked_agents() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    // trae is detected via ~/.trae and not linked; it holds a skill and a stray
    // file (classified the same way linking classifies them).
    std::fs::create_dir_all(home.join(".trae/skills/docx")).unwrap();
    std::fs::write(
        home.join(".trae/skills/docx/SKILL.md"),
        "---\nname: docx\ndescription: does docx\n---\nbody",
    )
    .unwrap();
    std::fs::write(home.join(".trae/skills/notes.txt"), "x").unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    let statuses = manager.agent_status(true);
    let trae = statuses.iter().find(|s| s.name == "trae").unwrap();
    assert!(!trae.canonical);
    assert!(!trae.linked);
    assert_eq!(trae.internal_skills, vec!["docx".to_string()]);
    assert_eq!(trae.internal_others, vec!["notes.txt".to_string()]);
}

#[test]
fn lib_agent_link_adopts_and_unlink_keeps_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    // trae is detected via ~/.trae; give it a skill and a stray file.
    std::fs::create_dir_all(home.join(".trae/skills/docx")).unwrap();
    std::fs::write(
        home.join(".trae/skills/docx/SKILL.md"),
        "---\nname: docx\ndescription: does docx\n---\nbody",
    )
    .unwrap();
    std::fs::write(home.join(".trae/skills/README.txt"), "x").unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    // Link: the skill is adopted, the stray file is quarantined, the dir becomes a link.
    let outcome = manager
        .agent(&AgentRequest {
            agents: vec!["trae".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();
    match &outcome.results[0].outcome {
        LinkOutcome::Linked {
            adopted,
            quarantined,
            conflicts,
        } => {
            assert_eq!(adopted, &["docx".to_string()]);
            assert_eq!(quarantined, &["README.txt".to_string()]);
            assert!(conflicts.is_empty());
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert!(home.join(".trae/skills").is_symlink());
    assert!(home.join(".agents/skills/docx/SKILL.md").exists());
    assert!(
        home.join(".agents/skills/.misc/trae/README.txt").exists(),
        "non-skill files are quarantined under .misc/"
    );

    // Status reports the agent as linked.
    let statuses = manager.agent_status(true);
    let trae = statuses.iter().find(|s| s.name == "trae").unwrap();
    assert!(trae.linked);

    // Unlink recreates an empty real dir; adopted content stays canonical.
    let outcome = manager
        .agent(&AgentRequest {
            agents: vec!["trae".to_string()],
            global: true,
            unlink: true,
        })
        .unwrap();
    assert!(matches!(outcome.results[0].outcome, LinkOutcome::Unlinked));
    assert!(home.join(".agents/skills/docx/SKILL.md").exists());
    let dir = home.join(".trae/skills");
    assert!(dir.is_dir());
    assert!(!dir.is_symlink());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn lib_agent_status_orders_canonical_first() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    // At global scope cline is canonical (global dir ~/.agents/skills);
    // claude-code (non-canonical) precedes cline in the static agent table, so
    // this fixture proves the canonical-first ordering rather than a coincidence.
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".cline")).unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        // Hermetic: never probe system locations (e.g. /Applications/ZCode.app).
        .probe_system_dirs(false)
        .build();

    manager
        .agent(&AgentRequest {
            agents: vec!["claude-code".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();

    let statuses = manager.agent_status(true);
    let names: Vec<&str> = statuses.iter().map(|s| s.name.as_str()).collect();
    // canonical agents come first; the rest keep table order.
    assert_eq!(names, vec!["cline", "claude-code"]);
}

#[test]
fn lib_disable_then_enable_roundtrip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();

    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    // Add a local skill.
    let src = write_skill_source(tmp.path(), "src", "pdf");
    manager
        .add(&AddRequest {
            source: src.display().to_string(),
            ..Default::default()
        })
        .unwrap();

    // Disable: the dir moves out of the canonical dir into disabled-skills.
    let disabled = manager
        .disable(&DisableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(disabled.disabled, vec!["pdf".to_string()]);
    assert!(disabled.already.is_empty());
    assert!(disabled.missing.is_empty());
    assert!(!cwd.join(".agents/skills/pdf").exists());
    assert!(cwd.join(".agents/disabled-skills/pdf/SKILL.md").exists());

    // list reports the skill as disabled.
    let listed = manager.list(&ListRequest::default()).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].enabled);

    // Enable: the dir moves back into the canonical dir.
    let enabled = manager
        .enable(&EnableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(enabled.enabled, vec!["pdf".to_string()]);
    assert!(cwd.join(".agents/skills/pdf/SKILL.md").exists());
    assert!(!cwd.join(".agents/disabled-skills/pdf").exists());

    let listed = manager.list(&ListRequest::default()).unwrap();
    assert!(listed[0].enabled);
}

#[test]
fn lib_disable_enable_are_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();

    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();

    let src = write_skill_source(tmp.path(), "src", "pdf");
    manager
        .add(&AddRequest {
            source: src.display().to_string(),
            ..Default::default()
        })
        .unwrap();

    // Disabling twice: the second call reports "already disabled", not an error.
    manager
        .disable(&DisableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    let again = manager
        .disable(&DisableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert!(again.disabled.is_empty());
    assert_eq!(again.already, vec!["pdf".to_string()]);

    // Enabling twice: the second call reports "already enabled".
    manager
        .enable(&EnableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    let again = manager
        .enable(&EnableRequest {
            skills: vec!["pdf".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert!(again.enabled.is_empty());
    assert_eq!(again.already, vec!["pdf".to_string()]);

    // Missing names are reported via `missing`, never errors.
    let missing = manager
        .disable(&DisableRequest {
            skills: vec!["nope".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert!(missing.disabled.is_empty());
    assert!(missing.already.is_empty());
    assert_eq!(missing.missing, vec!["nope".to_string()]);
}

#[test]
fn lib_disable_global_scope_moves_home_skill() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let manager = Manager::builder()
        .home(home.clone())
        .config(tmp.path().join("config"))
        .cwd(tmp.path().join("project"))
        .build();

    let src = write_skill_source(tmp.path(), "src", "pdf");
    manager
        .add(&AddRequest {
            source: src.display().to_string(),
            global: true,
            ..Default::default()
        })
        .unwrap();

    manager
        .disable(&DisableRequest {
            skills: vec!["pdf".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();

    assert!(!home.join(".agents/skills/pdf").exists());
    assert!(home.join(".agents/disabled-skills/pdf/SKILL.md").exists());
}
