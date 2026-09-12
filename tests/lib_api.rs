//! Integration tests for the library API: proving the lib works independently of the CLI,
//! driven entirely through `ManagerBuilder` + request structs (no `assert_cmd`).

use std::path::{Path, PathBuf};

use agents_skills::{
    AddRequest, AgentRequest, DisableRequest, EnableRequest, LinkOutcome, ListRequest, Manager,
    RemoveRequest, SkillsError, UpdateRequest,
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

    // List finds it, with scope and lock metadata.
    let listed = manager.list(&ListRequest::default()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "pdf");
    assert_eq!(listed[0].scope, "project");
    assert!(listed[0].source.is_some());

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
    assert!(json.contains("\"scope\": \"project\""));
    assert!(json.contains("\"source\":"));
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

    // trae is detected via ~/.trae and not linked; it holds an internal skill
    // and a stray file (classified the same way link/migrate classify).
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
    assert!(trae.pending_backup.is_none());
}

#[test]
fn lib_agent_link_parks_and_unlink_restores() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    // trae is detected via ~/.trae; give it a pre-existing skill and a stray file.
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

    // Plain link: content is parked in the backup slot, the dir becomes a link.
    let outcome = manager
        .agent(&AgentRequest {
            agents: vec!["trae".to_string()],
            global: true,
            ..Default::default()
        })
        .unwrap();
    match &outcome.results[0].outcome {
        LinkOutcome::Linked {
            parked_skills,
            parked_others,
            backup_dir,
        } => {
            assert_eq!(parked_skills, &["docx".to_string()]);
            assert_eq!(parked_others, &["README.txt".to_string()]);
            assert_eq!(
                backup_dir.as_deref(),
                Some(home.join(".agents/backup-skills/trae/skills").as_path())
            );
        }
        other => panic!("expected Linked, got {other:?}"),
    }
    assert!(home.join(".trae/skills").is_symlink());
    assert!(
        !home.join(".agents/skills").exists(),
        "plain link never migrates"
    );

    // Status reports the agent as linked; the slot only shows while unlinked.
    let statuses = manager.agent_status(true);
    let trae = statuses.iter().find(|s| s.name == "trae").unwrap();
    assert!(trae.linked);

    // Unlink restores the parked content into a real dir and drops the slot.
    let outcome = manager
        .agent(&AgentRequest {
            agents: vec!["trae".to_string()],
            global: true,
            unlink: true,
            ..Default::default()
        })
        .unwrap();
    match &outcome.results[0].outcome {
        LinkOutcome::Unlinked {
            restored,
            restored_from,
        } => {
            assert_eq!(restored.len(), 2);
            assert_eq!(
                restored_from.as_deref(),
                Some(home.join(".agents/backup-skills/trae/skills").as_path())
            );
        }
        other => panic!("expected Unlinked, got {other:?}"),
    }
    assert!(home.join(".trae/skills/docx/SKILL.md").exists());
    assert!(home.join(".trae/skills/README.txt").exists());
    assert!(!home.join(".agents/backup-skills/trae").exists());
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
fn lib_list_agent_filter_and_visibility() {
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

    // All universal agents see the skill by default (agents holds display names).
    let all = manager.list(&ListRequest::default()).unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].agents.contains(&"Codex".to_string()));

    // Filtering to a universal agent keeps the skill visible.
    let filtered = manager
        .list(&ListRequest {
            agents: vec!["codex".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert!(filtered[0].agents.contains(&"Codex".to_string()));

    // Filtering to an uninstalled, unlinked agent keeps the skill listed but
    // reports no visible agents (matches the CLI's -a behavior).
    let none = manager
        .list(&ListRequest {
            agents: vec!["claude-code".to_string()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(none.len(), 1);
    assert!(none[0].agents.is_empty());
}

#[test]
fn lib_update_empty_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .cwd(tmp.path().join("project"))
        .build();

    let outcome = manager.update(&UpdateRequest::default()).unwrap();
    assert_eq!(outcome.updated, 0);
    assert_eq!(outcome.failed, 0);
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

#[test]
fn lib_update_reports_install_failures_without_updated_names() {
    let tmp = tempfile::TempDir::new().unwrap();

    // A real git repo so the lock records a non-local (git) source: `update` re-fetches it.
    let repo_dir = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo = git2::Repository::init(&repo_dir).unwrap();
    std::fs::write(
        repo_dir.join("SKILL.md"),
        "---\nname: pdf\ndescription: does pdf\n---\n\n# pdf\n",
    )
    .unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("SKILL.md")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();

    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let manager = Manager::builder()
        .home(tmp.path().join("home"))
        .config(tmp.path().join("config"))
        .cwd(cwd.clone())
        .build();
    manager
        .add(&AddRequest::new(format!("file://{}", repo_dir.display())))
        .unwrap();

    // Sabotage the canonical install: a file sits where the skill dir should be,
    // so reinstalling fails inside install_skill (remove_dir_all on a file).
    let canonical = cwd.join(".agents/skills/pdf");
    std::fs::remove_dir_all(&canonical).unwrap();
    std::fs::write(&canonical, "not a dir").unwrap();

    let outcome = manager.update(&UpdateRequest::default()).unwrap();

    assert_eq!(outcome.updated, 0);
    assert_eq!(outcome.failed, 1);
    assert!(
        outcome.updated_names.is_empty(),
        "failed skills must not be reported as updated"
    );
    assert_eq!(outcome.failures.len(), 1);
    assert!(outcome.failures[0].starts_with("pdf:"));
}
