//! High-level `Manager` facade: one-stop add/list/remove over an injectable [`Env`].
//!
//! The manager is pure data: it returns structured outcomes and never prints or exits;
//! the CLI layer (src/commands) is responsible for rendering.

use std::path::PathBuf;

use crate::core::agents::{
    AGENTS, Env, canonical_skills_dir, config_home, disabled_skills_dir, home, is_installed,
    is_native,
};
use crate::core::discover::{Skill, read_skill};
use crate::core::github::fetch_skill;
use crate::core::install::{
    install_skill, list_disabled_skills, list_installed_skills, remove_skill, scan_disabled,
    scan_installed,
};
use crate::core::link::{is_agent_linked, link_agent, private_content, unlink_agent};
use crate::core::source::{SourceType, parse_source};
use crate::error::{Result, SkillsError};

use crate::manager::select::{resolve_target_agents, resolve_to_remove, set_enabled_state};
pub use crate::manager::types::{
    AddOutcome, AddRequest, AgentLinkResult, AgentOutcome, AgentRequest, AgentStatus,
    DisableOutcome, DisableRequest, EnableOutcome, EnableRequest, ListedSkill, RemoveOutcome,
    RemoveRequest,
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
/// let req = AddRequest::new("anthropics/skills@pdf");
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

    /// Add (install) exactly one skill from a source.
    ///
    /// `source` is either a local skill directory (it must directly contain a
    /// `SKILL.md`) or `owner/repo@<skill>`, naming one skill on GitHub. The
    /// skill name is always its **directory name**: for GitHub sources it
    /// matches, case-insensitively, a repository directory that directly
    /// contains `SKILL.md` (shallowest match wins); a `SKILL.md` at the
    /// repository root is selected with the repository name. The frontmatter
    /// `name` is ignored. Pin a branch, tag, or commit SHA with
    /// [`AddRequest::reference`]; otherwise the repository's default branch is
    /// used. Remote installs go through the GitHub API, which downloads only the
    /// matched skill directory — set `GITHUB_TOKEN` to raise its rate limit
    /// (60 → 5000 requests/hour).
    ///
    /// The skill is installed into the canonical dir (the only place real files
    /// live). `add` only ever adds: when a skill of the same name is already
    /// installed — enabled *or* disabled — [`AddOutcome::skipped`] is `true` and
    /// the existing copy is left untouched, so local edits are never silently
    /// discarded. Replace an installed skill with [`Manager::remove`] followed by
    /// `add`.
    ///
    /// `add` never links any agent: use [`Manager::agent`] to expose the canonical
    /// dir to an agent afterwards.
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
    /// assert_eq!(outcome.skill.name, "hello");
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// - [`SkillsError::Message`] when the source syntax is invalid, the local
    ///   directory is missing or has no direct `SKILL.md`, or the named skill
    ///   does not exist in the GitHub repository.
    /// - [`SkillsError::Http`] / [`SkillsError::Io`] for transport and filesystem
    ///   failures.
    pub fn add(&self, req: &AddRequest) -> Result<AddOutcome> {
        let parsed = parse_source(&req.source)?;

        // Resolve exactly one skill. The temp dir backing a remote fetch is held
        // until the install below finishes (it drops and cleans up afterwards).
        let skill: Skill;
        let _temp: Option<tempfile::TempDir>;
        match parsed.ty {
            SourceType::Local => {
                if req.reference.is_some() {
                    return Err(SkillsError::msg(
                        "--ref can only pin a GitHub source (`owner/repo@<skill>`).",
                    ));
                }
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
                if !path.join("SKILL.md").is_file() {
                    return Err(SkillsError::msg(format!(
                        "Not a skill directory: \"{}\" must directly contain a SKILL.md file.",
                        path.display()
                    )));
                }
                // Identity is the directory name; the manifest only provides the
                // description. A skill named by its explicit path may be internal.
                let Some(s) = read_skill(path, true) else {
                    return Err(SkillsError::msg(format!(
                        "Not a skill directory: \"{}\" has no usable directory name.",
                        path.display()
                    )));
                };
                skill = s;
                _temp = None;
            }
            SourceType::Github => {
                let (tmp, s) = fetch_skill(
                    &parsed.owner,
                    &parsed.repo,
                    &parsed.skill,
                    req.reference.as_deref(),
                )?;
                skill = s;
                _temp = Some(tmp);
            }
        }

        // Install into the canonical dir (the only place real files live).
        // An already-installed name (enabled or disabled) is skipped, not replaced.
        let result = install_skill(&skill, &self.env);
        if !result.success {
            return Err(SkillsError::msg(
                result.error.unwrap_or_else(|| "install failed".to_string()),
            ));
        }

        Ok(AddOutcome {
            source: parsed,
            skill,
            canonical_path: result.canonical_path,
            skipped: result.skipped,
        })
    }

    /// Link or unlink agents' skills dirs relative to the canonical dir.
    ///
    /// Connects each agent's own skills dir to the canonical dir with a
    /// directory-level symlink, so every install/update/remove is immediately
    /// visible to all linked agents. With `req.unlink`, disconnects those dirs
    /// instead — removes the symlink (only when it points at the canonical dir)
    /// and recreates an empty dir; the canonical dir and its skills are left
    /// untouched.
    ///
    /// Linking *adopts* whatever the agent dir already holds, and that is not
    /// reversible: skill dirs are moved into the canonical dir, non-skill entries
    /// are quarantined under `.misc/<agent>/` inside it, and name clashes are
    /// dropped in favour of the existing copy — the canonical copy wins, and a
    /// name disabled in the `disabled-skills` dir stays disabled instead of being
    /// re-imported (both reported via [`LinkOutcome::Linked`] `conflicts`). Legacy
    /// per-skill symlinks pointing into the canonical dir are dropped as well,
    /// since their content already lives there. Adopted content is managed by
    /// [`Manager::remove`] / [`Manager::disable`] from then on; unlinking does not
    /// move it back.
    ///
    /// Linking is refused only when the agent dir is a symlink pointing somewhere
    /// other than the canonical dir.
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
                    unlink_agent(agent, &self.env)
                } else {
                    link_agent(agent, &self.env)
                },
            })
            .collect();
        Ok(AgentOutcome { results })
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
    /// private content (`internal_skills` / `internal_others`) — what linking
    /// would adopt into the canonical dir.
    ///
    /// Ordering: agents that natively use the canonical dir (`canonical: true`)
    /// come first, then the remaining agents — both groups keep the static agent
    /// table order. This is the exact order `agent --status` renders; callers do
    /// not need to sort again.
    pub fn agent_status(&self) -> Vec<AgentStatus> {
        let mut statuses: Vec<AgentStatus> = AGENTS
            .iter()
            .filter(|a| {
                is_installed(a, &self.env)
                    || (!is_native(a, &self.env) && is_agent_linked(a, &self.env))
            })
            .map(|a| {
                let canonical = is_native(a, &self.env);
                let linked = is_agent_linked(a, &self.env);
                // For unlinked, non-canonical agents, classify the private content
                // of the agent's own skills dir (canonical/linked agents share the
                // canonical dir, whose contents are shown by `list` instead).
                let (internal_skills, internal_others) = if linked || canonical {
                    (Vec::new(), Vec::new())
                } else {
                    private_content(a, &self.env)
                };
                AgentStatus {
                    name: a.name.to_string(),
                    display: a.display.to_string(),
                    linked,
                    canonical,
                    internal_skills,
                    internal_others,
                }
            })
            .collect();
        // Stable sort: canonical agents first, others keep table order.
        statuses.sort_by_key(|s| !s.canonical);
        statuses
    }

    /// List installed skills.
    ///
    /// Scans the canonical skills directory (plus the disabled dir), producing
    /// serde-serializable [`ListedSkill`] values — the same shape emitted by
    /// `list --json`. Which agents see a skill is not per-skill: every linked or
    /// native agent sees all skills in the canonical dir. Use
    /// [`Manager::agent_status`] to inspect that.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::Manager;
    ///
    /// let manager = Manager::new();
    /// let skills = manager.list()?;
    /// for skill in &skills {
    ///     println!("{} -> {}", skill.name, manager.skill_dir(skill).display());
    /// }
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    pub fn list(&self) -> Result<Vec<ListedSkill>> {
        let installed = list_installed_skills(&self.env);
        let disabled = list_disabled_skills(&self.env);

        let mut out = Vec::new();
        for s in installed {
            out.push(ListedSkill {
                name: s.name,
                description: s.description,
                enabled: true,
                installed_at: s.installed_at,
            });
        }
        for s in disabled {
            out.push(ListedSkill {
                name: s.name,
                description: s.description,
                enabled: false,
                installed_at: s.installed_at,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// The on-disk directory of a listed skill.
    ///
    /// Resolves [`ListedSkill::name`] (already the on-disk directory name)
    /// against the canonical dir when the skill is enabled, or the sibling
    /// `disabled-skills` dir when it is disabled.
    ///
    /// [`ListedSkill::name`]: crate::ListedSkill::name
    pub fn skill_dir(&self, skill: &ListedSkill) -> PathBuf {
        let base = if skill.enabled {
            canonical_skills_dir(&self.env)
        } else {
            disabled_skills_dir(&self.env)
        };
        base.join(&skill.name)
    }

    /// Disable installed skills.
    ///
    /// Moves each selected skill's directory from the canonical dir into the sibling
    /// `disabled-skills` dir, hiding it from every linked or universal agent at once.
    /// Files are preserved, so [`Manager::enable`] restores them losslessly.
    ///
    /// A copy of the same skill already parked in the disabled dir is stale — a
    /// disabled skill can be re-installed behind our back by a third-party agent —
    /// so the copy being moved wins and the stale one is discarded. One skill name
    /// therefore always maps to exactly one directory.
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
    /// let skill_dir = tmp.path().join("home/.agents/skills/pdf");
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
        let installed = scan_installed(&self.env);
        let disabled = scan_disabled(&self.env);

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
            set_enabled_state(&requested, &installed, &disabled, false, &self.env)?;

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
    /// This is the exact inverse of [`Manager::disable`], except for which copy
    /// survives a clash: an enabled copy of the same skill — a third-party agent may
    /// re-install a skill whose copy is still parked — is discarded and replaced by
    /// the copy being moved. A name is never left present in both dirs.
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
    /// let skill_dir = tmp.path().join("home/.agents/disabled-skills/pdf");
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
        let disabled = scan_disabled(&self.env);
        let installed = scan_installed(&self.env);

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
            set_enabled_state(&requested, &disabled, &installed, true, &self.env)?;

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
    /// Deletes each skill's directory from the canonical dir, plus any parked copy in
    /// the disabled dir — including copies under a differently normalized directory
    /// name. Removal applies to every linked agent at once (they all share the
    /// canonical dir); agent links themselves are untouched — call [`Manager::agent`]
    /// with `unlink: true` to disconnect an agent instead.
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
        // Disabled skills are still installed (parked in `disabled-skills`): scan them
        // too so `remove <name>` and `remove --all` can find and delete them.
        let installed = scan_installed(&self.env);
        let disabled = scan_disabled(&self.env);

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

        // Remove every copy of each selected name: the canonical one (visible to
        // every linked agent at once) and any parked copy in the disabled dir,
        // including copies under a differently normalized directory name.
        let mut removed: Vec<String> = Vec::new();
        for name in &selected {
            if remove_skill(name, &self.env) {
                removed.push(name.clone());
            }
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
