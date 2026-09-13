//! High-level `Manager` facade: one-stop add/list/remove over an injectable [`Env`].
//!
//! The manager is pure data: it returns structured outcomes and never prints or exits;
//! the CLI layer (src/commands) is responsible for rendering.

use std::path::PathBuf;

use crate::core::agents::{
    AGENTS, Env, agent_display, config_home, disabled_skills_dir, get_agent, home, is_installed,
    is_native,
};
use crate::core::discover::{Skill, discover_skills, filter_skills};
use crate::core::fetch::fetch_source;
use crate::core::github::{fetch_skill_via_api, fetch_subdir_via_api};
use crate::core::install::{
    get_canonical_path, install_skill, list_disabled_skills, list_installed_skills, sanitize_name,
    scan_disabled, scan_installed,
};
use crate::core::link::{
    is_agent_linked, link_agent, pending_backup, private_content, unlink_agent,
};
use crate::core::source::{SourceType, parse_source};
use crate::error::{Result, SkillsError};

use crate::manager::select::{
    resolve_target_agents, resolve_to_remove, set_enabled_state, skill_filters,
};
pub use crate::manager::types::{
    AddOutcome, AddRequest, AgentLinkResult, AgentOutcome, AgentRequest, AgentStatus, BackupStatus,
    DisableOutcome, DisableRequest, EnableOutcome, EnableRequest, InstallFailure, InstallSuccess,
    ListRequest, ListedSkill, RemoveOutcome, RemoveRequest,
};
mod select;
#[cfg(test)]
mod tests;
mod types;

/// Skill manager: carries injectable context and runs add/list/remove/enable/disable.
///
/// This is the high-level entry point for library consumers. It resolves an [`Env`]
/// (home / config / cwd) once at construction, then every operation is a plain method
/// taking a request struct and returning a structured outcome.
///
/// # Examples
///
/// ```
/// use agents_skills::{AddRequest, Manager};
///
/// // Real environment:
/// let real = Manager::new();
///
/// // Or a sandboxed environment (no side effects outside the given paths):
/// let sandboxed = Manager::builder()
///     .home("/tmp/home")
///     .config("/tmp/config")
///     .cwd("/tmp/project")
///     .build();
///
/// let req = AddRequest::new("anthropics/skills");
/// let _ = (real, sandboxed, req);
/// ```
pub struct Manager {
    env: Env,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Manager {
    /// Build a manager from the real environment (home / config / cwd).
    ///
    /// Equivalent to [`Manager::builder`]`().build()`.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Start customizing a manager (inject home/config/cwd/env vars).
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::Manager;
    ///
    /// let manager = Manager::builder()
    ///     .home("/tmp/home")
    ///     .env_var("CLAUDE_CONFIG_DIR", "/tmp/claude")
    ///     .build();
    /// ```
    pub fn builder() -> ManagerBuilder {
        ManagerBuilder::default()
    }

    /// Access the resolved environment context.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Add (install) skills from a source.
    ///
    /// Parses the source, discovers its skills, and installs each selected skill
    /// into the canonical dir (the only place real files live). Returns a
    /// structured [`AddOutcome`] with discovered, selected, installed and failed
    /// skills.
    ///
    /// `add` never links any agent: use [`Manager::agent`] to expose the canonical
    /// dir to an agent afterwards.
    ///
    /// # Selection defaults
    ///
    /// - `skills` empty → all discovered skills; a `"*"` entry → all as well.
    /// - `list_only` → discover and report, without installing anything.
    ///
    /// # Examples
    ///
    /// Install a local skill into a scratch environment (hermetic — no network, no
    /// real home access):
    ///
    /// ```
    /// use agents_skills::{AddRequest, Manager};
    ///
    /// let tmp = tempfile::TempDir::new().unwrap();
    /// let src = tmp.path().join("hello");
    /// std::fs::create_dir_all(&src).unwrap();
    /// std::fs::write(
    ///     src.join("SKILL.md"),
    ///     "---\nname: hello\ndescription: says hello\n---\n\n# hello\n",
    /// )
    /// .unwrap();
    ///
    /// let manager = Manager::builder()
    ///     .home(tmp.path().join("home"))
    ///     .config(tmp.path().join("config"))
    ///     .cwd(tmp.path().join("project"))
    ///     .build();
    ///
    /// let outcome = manager.add(&AddRequest::new(src.display().to_string()))?;
    /// assert!(!outcome.installed.is_empty());
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - [`SkillsError::Message`] when the source is invalid, unreadable, or contains
    ///   no valid skill (a `SKILL.md` with `name` and `description`).
    /// - [`SkillsError::Git`], [`SkillsError::Http`], [`SkillsError::Io`],
    ///   [`SkillsError::Zip`], etc. for transport and filesystem failures.
    pub fn add(&self, req: &AddRequest) -> Result<AddOutcome> {
        let parsed = parse_source(&req.source)?;
        // `@skill` in the source is an explicit selection, like `--skill`.
        let include_internal = !req.skills.is_empty() || parsed.skill_filter.is_some();

        // Fetch skills (the temp dir is held until install finishes).
        let skills: Vec<Skill>;
        let _temp: Option<tempfile::TempDir>;
        if parsed.ty == SourceType::Local {
            let path = parsed
                .local_path
                .as_ref()
                .ok_or_else(|| SkillsError::msg("local source missing path"))?;
            if !path.exists() {
                return Err(SkillsError::msg(format!(
                    "Local path does not exist: {}",
                    path.display()
                )));
            }
            skills = discover_skills(path, parsed.subpath.as_deref(), include_internal)?;
            _temp = None;
        } else {
            // Fast path: fetch only what is needed via the GitHub API when
            // possible — the `@skill`-selected dir, or every file under the
            // subpath — falling back to the whole-repo archive on any failure.
            let fast: Option<(tempfile::TempDir, PathBuf)> =
                if let (Some(name), false) = (parsed.skill_filter.as_deref(), req.list_only) {
                    fetch_skill_via_api(&parsed, name, include_internal)
                        .ok()
                        .flatten()
                } else if parsed.ty == SourceType::Github {
                    fetch_subdir_via_api(&parsed).ok().flatten()
                } else {
                    None
                };
            let (tmp, root) = match fast {
                Some(v) => v,
                None => fetch_source(&parsed)?,
            };
            skills = discover_skills(&root, parsed.subpath.as_deref(), include_internal)?;
            _temp = Some(tmp);
        }

        if skills.is_empty() {
            return Err(SkillsError::msg(
                "No valid skills found. Skills require a SKILL.md with name and description.",
            ));
        }

        // --list: report discovered skills without installing.
        if req.list_only {
            return Ok(AddOutcome {
                source: parsed,
                skills,
                selected: Vec::new(),
                installed: Vec::new(),
                failed: Vec::new(),
                list_only: true,
            });
        }

        // Select skills. `--skill` args and the source's `@skill` filter both count.
        let filters = skill_filters(&req.skills, parsed.skill_filter.as_deref());
        let selected: Vec<Skill> = if filters.iter().any(|s| s == "*") {
            skills.clone()
        } else if !filters.is_empty() {
            filter_skills(&skills, &filters)
        } else {
            skills.clone()
        };

        // Install into the canonical dir (the only place real files live).
        let mut installed: Vec<InstallSuccess> = Vec::new();
        let mut failed: Vec<InstallFailure> = Vec::new();
        for skill in &selected {
            let r = install_skill(skill, req.global, &self.env);
            if r.success && !r.skipped {
                installed.push(InstallSuccess {
                    name: skill.name.clone(),
                    canonical_path: r.canonical_path,
                });
            } else if !r.success {
                failed.push(InstallFailure {
                    skill: skill.name.clone(),
                    error: r.error.unwrap_or_default(),
                });
            }
        }

        Ok(AddOutcome {
            source: parsed,
            skills,
            selected,
            installed,
            failed,
            list_only: false,
        })
    }

    /// Link or unlink agents' skills dirs relative to the canonical dir.
    ///
    /// Connects each agent's own skills dir to the canonical dir with a
    /// directory-level symlink, so every install/update/remove is immediately
    /// visible to all linked agents. With `req.unlink`, disconnects those dirs
    /// instead — removes the symlink (only when it points at the canonical dir)
    /// and restores any parked backup content into a real dir; the canonical dir
    /// and its skills are left untouched.
    ///
    /// Pre-existing content is never destroyed. When linking, every entry of the
    /// agent dir that does not go into the canonical dir is parked in a backup
    /// slot (`<base>/.agents/backup-skills/<agent>`); unlink restores it. With
    /// `req.migrate`, skill subdirs are moved into the canonical dir instead —
    /// name clashes keep the canonical copy, and names disabled in the
    /// `disabled-skills` dir stay disabled (the agent-side copy is parked,
    /// reported via [`LinkOutcome::Migrated`] `skipped`) — and only non-skill
    /// entries are parked. Rerunning with `migrate` on an already linked agent
    /// pulls parked skills out of the backup slot. Legacy per-skill symlinks
    /// pointing into the canonical dir are taken over automatically. Linking is
    /// refused only when the agent dir is a foreign symlink or a stale non-empty
    /// backup slot exists.
    ///
    /// Agents native to the requested scope (whose skills dir already is that
    /// scope's canonical dir) report [`LinkOutcome::AlreadyLinked`]. Nativeness
    /// is scope-aware: e.g. Antigravity is native at project scope
    /// (`.agents/skills`) but linked at global scope (`~/.gemini/config/skills`).
    /// Agents whose root dir does not exist in this scope are reported as
    /// [`LinkOutcome::Skipped`] (except `claude-code`, the historical exception).
    ///
    /// # Selection defaults
    ///
    /// - `agents` empty → auto-detect installed agents (plus the universal agents);
    ///   a `"*"` entry → every known agent.
    ///
    /// # Errors
    ///
    /// [`SkillsError::InvalidAgents`] when `agents` names an unknown agent.
    pub fn agent(&self, req: &AgentRequest) -> Result<AgentOutcome> {
        let target_agents = resolve_target_agents(&req.agents, &self.env)?;
        let results = target_agents
            .iter()
            .map(|agent| AgentLinkResult {
                agent: agent.name.to_string(),
                display: agent.display.to_string(),
                outcome: if req.unlink {
                    unlink_agent(agent, req.global, &self.env)
                } else {
                    link_agent(agent, req.global, &self.env, req.migrate)
                },
            })
            .collect();
        Ok(AgentOutcome {
            global: req.global,
            results,
        })
    }

    /// Link status of every installed agent in this scope.
    ///
    /// Only agents detected as installed locally (or already linked) are reported.
    /// Agents that natively read this scope's canonical dir report `canonical`;
    /// agents connected via a directory-level symlink report `linked`. Both
    /// classifications are scope-aware (an agent may be canonical at project
    /// scope but linked at global scope).
    ///
    /// For unlinked, non-canonical agents the status classifies the agent dir's
    /// private content (`internal_skills` / `internal_others`, the same rules
    /// link and migrate use) and reports a pending backup slot (`pending_backup`)
    /// when one is waiting to be restored by unlink.
    ///
    /// Ordering: agents that natively use the canonical dir (`canonical: true`)
    /// come first, then the remaining agents — both groups keep the static agent
    /// table order. This is the exact order `agent --status` renders; callers do
    /// not need to sort again.
    pub fn agent_status(&self, global: bool) -> Vec<AgentStatus> {
        let mut statuses: Vec<AgentStatus> = AGENTS
            .iter()
            .filter(|a| {
                is_installed(a, &self.env)
                    || (!is_native(a, global, &self.env) && is_agent_linked(a, global, &self.env))
            })
            .map(|a| {
                let canonical = is_native(a, global, &self.env);
                let linked = is_agent_linked(a, global, &self.env);
                // For unlinked, non-canonical agents, classify the private content
                // of the agent's own skills dir (canonical/linked agents share the
                // canonical dir, whose contents are shown by `list` instead).
                let (internal_skills, internal_others, pending_backup) = if linked || canonical {
                    (Vec::new(), Vec::new(), None)
                } else {
                    let (skills, others) = private_content(a, global, &self.env);
                    let backup = pending_backup(a, global, &self.env)
                        .map(|(path, items)| BackupStatus { path, items });
                    (skills, others, backup)
                };
                AgentStatus {
                    name: a.name.to_string(),
                    display: a.display.to_string(),
                    linked,
                    canonical,
                    internal_skills,
                    internal_others,
                    pending_backup,
                }
            })
            .collect();
        // Stable sort: canonical agents first, others keep table order.
        statuses.sort_by_key(|s| !s.canonical);
        statuses
    }

    /// List installed skills (project or global).
    ///
    /// Scans the canonical skills directory (plus the disabled dir), producing
    /// serde-serializable [`ListedSkill`] values — the same shape emitted by
    /// `list --json`.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{ListRequest, Manager};
    ///
    /// let manager = Manager::new();
    /// let skills = manager.list(&ListRequest::default())?;
    /// for skill in skills {
    ///     println!("{} -> {}", skill.name, skill.path.display());
    /// }
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`SkillsError::InvalidAgents`] when `agents` names an unknown agent.
    pub fn list(&self, req: &ListRequest) -> Result<Vec<ListedSkill>> {
        let invalid: Vec<String> = req
            .agents
            .iter()
            .filter(|a| get_agent(a).is_none())
            .cloned()
            .collect();
        if !invalid.is_empty() {
            return Err(SkillsError::InvalidAgents(invalid.join(", ")));
        }

        let installed = list_installed_skills(&self.env, req.global, &req.agents);
        let disabled = list_disabled_skills(&self.env, req.global);

        let mut out = Vec::new();
        for s in &installed {
            out.push(ListedSkill {
                name: s.name.clone(),
                path: s.canonical_path.clone(),
                scope: s.scope.clone(),
                agents: s.agents.iter().map(|a| agent_display(a)).collect(),
                enabled: true,
            });
        }
        for s in &disabled {
            out.push(ListedSkill {
                name: s.name.clone(),
                path: s.canonical_path.clone(),
                scope: s.scope.clone(),
                agents: Vec::new(),
                enabled: false,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Disable installed skills.
    ///
    /// Moves each selected skill's directory from the canonical dir into the sibling
    /// `disabled-skills` dir, hiding it from every linked or universal agent at once.
    /// Files are preserved, so [`Manager::enable`] restores them losslessly.
    ///
    /// # Selection semantics
    ///
    /// - `skills` empty and `all` false → nothing is disabled; the outcome reports the
    ///   currently enabled names (used by the CLI to print a hint).
    /// - `all` true → every currently enabled skill.
    ///
    /// # Examples
    ///
    /// Disable an installed skill in a scratch environment (hermetic — no real
    /// home access):
    ///
    /// ```
    /// use agents_skills::{DisableRequest, Manager};
    ///
    /// let tmp = tempfile::TempDir::new().unwrap();
    /// // Simulate an installed skill in the canonical dir.
    /// let skill_dir = tmp.path().join("project/.agents/skills/pdf");
    /// std::fs::create_dir_all(&skill_dir).unwrap();
    /// std::fs::write(
    ///     skill_dir.join("SKILL.md"),
    ///     "---\nname: pdf\ndescription: pdf tools\n---\n\n# pdf\n",
    /// )
    /// .unwrap();
    ///
    /// let manager = Manager::builder()
    ///     .home(tmp.path().join("home"))
    ///     .config(tmp.path().join("config"))
    ///     .cwd(tmp.path().join("project"))
    ///     .build();
    ///
    /// let outcome = manager.disable(&DisableRequest {
    ///     skills: vec!["pdf".into()],
    ///     ..Default::default()
    /// })?;
    /// assert_eq!(outcome.disabled, vec!["pdf".to_string()]);
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`SkillsError::Io`] if a directory move fails.
    pub fn disable(&self, req: &DisableRequest) -> Result<DisableOutcome> {
        let global = req.global;
        let installed = scan_installed(&self.env, global);
        let disabled = scan_disabled(&self.env, global);

        if req.skills.is_empty() && !req.all {
            return Ok(DisableOutcome {
                installed,
                requested: Vec::new(),
                disabled: Vec::new(),
                already: Vec::new(),
                missing: Vec::new(),
            });
        }

        let requested: Vec<String> = if req.all {
            installed.clone()
        } else {
            req.skills.clone()
        };
        let (disabled_out, already, missing) =
            set_enabled_state(&requested, &installed, &disabled, global, false, &self.env)?;

        Ok(DisableOutcome {
            installed,
            requested,
            disabled: disabled_out,
            already,
            missing,
        })
    }

    /// Enable previously disabled skills.
    ///
    /// Moves each selected skill's directory from the `disabled-skills` dir back into
    /// the canonical dir, restoring its visibility to every linked or universal agent.
    /// This is the exact inverse of [`Manager::disable`].
    ///
    /// # Selection semantics
    ///
    /// - `skills` empty and `all` false → nothing is enabled; the outcome reports the
    ///   currently disabled names (used by the CLI to print a hint).
    /// - `all` true → every currently disabled skill.
    ///
    /// # Examples
    ///
    /// Re-enable a disabled skill in a scratch environment (hermetic — no real
    /// home access):
    ///
    /// ```
    /// use agents_skills::{EnableRequest, Manager};
    ///
    /// let tmp = tempfile::TempDir::new().unwrap();
    /// // Simulate a disabled skill parked in the disabled-skills dir.
    /// let skill_dir = tmp.path().join("project/.agents/disabled-skills/pdf");
    /// std::fs::create_dir_all(&skill_dir).unwrap();
    /// std::fs::write(
    ///     skill_dir.join("SKILL.md"),
    ///     "---\nname: pdf\ndescription: pdf tools\n---\n\n# pdf\n",
    /// )
    /// .unwrap();
    ///
    /// let manager = Manager::builder()
    ///     .home(tmp.path().join("home"))
    ///     .config(tmp.path().join("config"))
    ///     .cwd(tmp.path().join("project"))
    ///     .build();
    ///
    /// let outcome = manager.enable(&EnableRequest {
    ///     skills: vec!["pdf".into()],
    ///     ..Default::default()
    /// })?;
    /// assert_eq!(outcome.enabled, vec!["pdf".to_string()]);
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`SkillsError::Io`] if a directory move fails.
    pub fn enable(&self, req: &EnableRequest) -> Result<EnableOutcome> {
        let global = req.global;
        let disabled = scan_disabled(&self.env, global);
        let installed = scan_installed(&self.env, global);

        if req.skills.is_empty() && !req.all {
            return Ok(EnableOutcome {
                disabled,
                requested: Vec::new(),
                enabled: Vec::new(),
                already: Vec::new(),
                missing: Vec::new(),
            });
        }

        let requested: Vec<String> = if req.all {
            disabled.clone()
        } else {
            req.skills.clone()
        };
        let (enabled_out, already, missing) =
            set_enabled_state(&requested, &disabled, &installed, global, true, &self.env)?;

        Ok(EnableOutcome {
            disabled,
            requested,
            enabled: enabled_out,
            already,
            missing,
        })
    }

    /// Remove installed skills.
    ///
    /// Deletes each skill's directory from the canonical dir. Removal applies to
    /// every linked agent at once (they all share the canonical dir); agent links
    /// themselves are untouched — call [`Manager::agent`] with `unlink: true` to
    /// disconnect an agent instead.
    ///
    /// # Selection semantics
    ///
    /// - `skills` empty and `all` false → nothing is removed; the outcome reports the
    ///   currently enabled names (used by the CLI to print a hint).
    /// - `all` true → every installed skill (enabled or disabled).
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{Manager, RemoveRequest};
    ///
    /// let tmp = tempfile::TempDir::new().unwrap();
    /// let manager = Manager::builder()
    ///     .home(tmp.path().join("home"))
    ///     .cwd(tmp.path().join("project"))
    ///     .build();
    ///
    /// let req = RemoveRequest {
    ///     skills: vec!["pdf".to_string()],
    ///     ..Default::default()
    /// };
    /// // Nothing installed in the scratch dir, so this is a harmless no-op.
    /// let outcome = manager.remove(&req)?;
    /// assert!(outcome.removed.is_empty());
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    pub fn remove(&self, req: &RemoveRequest) -> Result<RemoveOutcome> {
        let global = req.global;

        // Disabled skills are still installed (parked in `disabled-skills`): scan them
        // too so `remove <name>` and `remove --all` can find and delete them.
        let installed = scan_installed(&self.env, global);
        let disabled = scan_disabled(&self.env, global);

        // List-only mode (no skills and not --all).
        if req.skills.is_empty() && !req.all {
            return Ok(RemoveOutcome {
                installed,
                requested: Vec::new(),
                removed: Vec::new(),
            });
        }

        // Resolve the skill names to remove against the on-disk dir names.
        let requested: Vec<String> = if req.all {
            installed.iter().chain(disabled.iter()).cloned().collect()
        } else {
            req.skills.clone()
        };
        if requested.is_empty() {
            return Ok(RemoveOutcome {
                installed,
                requested: Vec::new(),
                removed: Vec::new(),
            });
        }

        let selected = resolve_to_remove(&requested, &installed, &disabled);
        if selected.is_empty() {
            return Ok(RemoveOutcome {
                installed,
                requested,
                removed: Vec::new(),
            });
        }

        // Remove from the canonical dir (visible to every linked agent at once).
        let mut removed: Vec<String> = Vec::new();
        for name in &selected {
            let canonical = get_canonical_path(name, global, &self.env);
            let sanitized = sanitize_name(name);
            let _ = std::fs::remove_dir_all(&canonical);
            // Also remove any parked copy in the disabled dir.
            let parked = disabled_skills_dir(global, &self.env).join(&sanitized);
            let _ = std::fs::remove_dir_all(&parked);

            removed.push(name.clone());
        }

        Ok(RemoveOutcome {
            installed,
            requested,
            removed,
        })
    }
}

/// Chained builder for [`Manager`], injecting home/config/cwd/env vars.
///
/// Every field is optional: unset fields fall back to the real environment at
/// [`build`](Self::build) time, so tests and sandboxes can override just the pieces
/// they care about.
#[derive(Default)]
pub struct ManagerBuilder {
    home: Option<PathBuf>,
    config: Option<PathBuf>,
    cwd: Option<PathBuf>,
    vars: std::collections::HashMap<String, String>,
    probe_system_dirs: Option<bool>,
}

impl ManagerBuilder {
    /// Override the home directory.
    ///
    /// Affects global skills (`~/.agents/skills`) and per-agent
    /// user-level skills directories.
    pub fn home(mut self, p: impl Into<PathBuf>) -> Self {
        self.home = Some(p.into());
        self
    }

    /// Override the config directory.
    ///
    /// Affects agent config lookup (e.g. `CLAUDE_CONFIG_DIR` resolution).
    pub fn config(mut self, p: impl Into<PathBuf>) -> Self {
        self.config = Some(p.into());
        self
    }

    /// Override the current working directory.
    ///
    /// Affects project-scope installs (`.agents/skills`) and
    /// scope auto-detection.
    pub fn cwd(mut self, p: impl Into<PathBuf>) -> Self {
        self.cwd = Some(p.into());
        self
    }

    /// Inject an environment variable override.
    ///
    /// Useful for redirecting agent-specific env vars (e.g. `CLAUDE_CONFIG_DIR`) that
    /// the agent directory mapping consults. Does not touch the real process env.
    pub fn env_var(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.vars.insert(k.into(), v.into());
        self
    }

    /// Toggle probing of well-known system locations during agent detection.
    ///
    /// Some agents are detected via system locations outside home/config/cwd
    /// (e.g. `/Applications/ZCode.app`). Pass `false` in tests and sandboxes so
    /// detection never consults the real machine. Default: probe.
    pub fn probe_system_dirs(mut self, probe: bool) -> Self {
        self.probe_system_dirs = Some(probe);
        self
    }

    /// Build the [`Manager`], resolving defaults from the real environment.
    ///
    /// Unset fields fall back to the actual home/config/cwd of the process.
    pub fn build(self) -> Manager {
        let cwd = self
            .cwd
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let mut env = Env::new(
            self.home.unwrap_or_else(home),
            self.config.unwrap_or_else(config_home),
            cwd,
        );
        if !self.vars.is_empty() {
            env.set_vars(self.vars);
        }
        if let Some(probe) = self.probe_system_dirs {
            env.set_probe_system_dirs(probe);
        }
        Manager { env }
    }
}
