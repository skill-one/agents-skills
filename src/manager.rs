//! High-level `Manager` facade: one-stop add/list/remove/update over an injectable [`Env`].
//!
//! The manager is pure data: it returns structured outcomes and never prints or exits;
//! the CLI layer (src/commands) is responsible for rendering.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use serde::Serialize;

use crate::core::agents::{
    AGENTS, Agent, Env, agent_display, config_home, detect_installed_agents, disabled_skills_dir,
    ensure_universal_agents, get_agent, home, is_installed,
};
use crate::core::discover::{Skill, discover_skills, filter_skills};
use crate::core::fetch::fetch_source;
use crate::core::github::{fetch_skill_via_api, fetch_subdir_via_api};
use crate::core::install::{
    get_canonical_path, install_skill, list_disabled_skills, list_installed_skills, move_skill,
    sanitize_name, scan_disabled, scan_installed,
};
use crate::core::link::{
    LinkOutcome, is_agent_linked, link_agent, pending_backup, private_content, unlink_agent,
};
use crate::core::lock::{
    LockEntry, compute_folder_hash, find_lock_entry, global_lock_path, local_lock_path,
    lock_fields, read_local_lock, write_local_lock,
};
use crate::core::source::{Source, SourceType, parse_source};
use crate::error::{Result, SkillsError};

/// Skill manager: carries injectable context and runs add/list/remove/update.
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
    /// into the canonical dir (the only place real files live), recording
    /// successful installs in the lockfile. Returns a structured [`AddOutcome`]
    /// with discovered, selected, installed and failed skills.
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

        // Write the lock (only for successfully installed skills).
        if !installed.is_empty() {
            write_lock(&parsed, &selected, &installed, req.global, &self.env)?;
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
    /// Universal agents (whose skills dir already is the canonical dir) report
    /// [`LinkOutcome::AlreadyLinked`]. Agents whose root dir does not exist in
    /// this scope are reported as [`LinkOutcome::Skipped`] (except
    /// `claude-code`, the historical exception).
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
    /// Agents that natively read the canonical dir (universal) report `canonical`;
    /// agents connected via a directory-level symlink report `linked`.
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
                    || (!a.is_universal() && is_agent_linked(a, global, &self.env))
            })
            .map(|a| {
                let linked = is_agent_linked(a, global, &self.env);
                let canonical = a.is_universal();
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

    /// List installed skills (project or global), enriched with lock metadata.
    ///
    /// Scans the canonical skills directory and joins each entry with its lockfile
    /// record, producing serde-serializable [`ListedSkill`] values — the same shape
    /// emitted by `list --json`.
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

        let lock = read_local_lock(&lock_path(&self.env, req.global));
        let installed = list_installed_skills(&self.env, req.global, &req.agents);
        let disabled = list_disabled_skills(&self.env, req.global);

        let mut out = Vec::new();
        for s in &installed {
            let entry = find_lock_entry(&lock, &s.name);
            out.push(ListedSkill {
                name: s.name.clone(),
                path: s.canonical_path.clone(),
                scope: s.scope.clone(),
                agents: s.agents.iter().map(|a| agent_display(a)).collect(),
                source: entry.map(|e| e.source.clone()),
                source_url: entry.and_then(|e| e.source_url.clone()),
                source_type: entry.map(|e| e.source_type.clone()),
                enabled: true,
            });
        }
        for s in &disabled {
            let entry = find_lock_entry(&lock, &s.name);
            out.push(ListedSkill {
                name: s.name.clone(),
                path: s.canonical_path.clone(),
                scope: s.scope.clone(),
                agents: Vec::new(),
                source: entry.map(|e| e.source.clone()),
                source_url: entry.and_then(|e| e.source_url.clone()),
                source_type: entry.map(|e| e.source_type.clone()),
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
    /// Files are preserved, so [`Manager::enable`] restores them losslessly; the
    /// lockfile entry is kept, so `list` still shows the skill's source metadata.
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
    /// Deletes each skill's directory from the canonical dir and drops its lockfile
    /// entry. Removal applies to every linked agent at once (they all share the
    /// canonical dir); agent links themselves are untouched — call [`Manager::agent`]
    /// with `unlink: true` to disconnect an agent instead.
    ///
    /// # Selection semantics
    ///
    /// - `skills` empty and `all` false → nothing is removed; the outcome reports the
    ///   currently enabled names (used by the CLI to print a hint).
    /// - `all` true → every installed skill (enabled or disabled) plus every lockfile key.
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
        // too so `remove <name>` and `remove --all` can find and delete them, even when
        // they have no lockfile entry (e.g. symlinked by a third-party tool).
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

        // Resolve the skill names to remove (lock keys take priority, then on-disk dir names).
        let lock = read_local_lock(&lock_path(&self.env, global));
        let lock_keys: Vec<String> = lock.skills.keys().cloned().collect();
        let requested: Vec<String> = if req.all {
            installed
                .iter()
                .chain(disabled.iter())
                .chain(lock_keys.iter())
                .cloned()
                .collect()
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

        let selected = resolve_to_remove(&requested, &installed, &disabled, &lock_keys);
        if selected.is_empty() {
            return Ok(RemoveOutcome {
                installed,
                requested,
                removed: Vec::new(),
            });
        }

        // Remove from the canonical dir (visible to every linked agent at once).
        let lock_path = lock_path(&self.env, global);
        let mut lock = read_local_lock(&lock_path);
        let mut removed: Vec<String> = Vec::new();
        for name in &selected {
            let canonical = get_canonical_path(name, global, &self.env);
            let sanitized = sanitize_name(name);
            let _ = std::fs::remove_dir_all(&canonical);
            // Also remove any parked copy in the disabled dir.
            let parked = disabled_skills_dir(global, &self.env).join(&sanitized);
            let _ = std::fs::remove_dir_all(&parked);

            // Clean the lock.
            lock.version = 1;
            lock.skills.remove(name);
            lock.skills.remove(&sanitized);

            removed.push(name.clone());
        }

        // Flush the lock once for all removals.
        if !removed.is_empty() {
            if let Some(parent) = lock_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = write_local_lock(&lock, &lock_path);
        }

        Ok(RemoveOutcome {
            installed,
            requested,
            removed,
        })
    }

    /// Update installed skills from their recorded (non-local) sources.
    ///
    /// Reads the lockfile, re-clones each recorded source once (skills sharing a source
    /// are grouped), re-installs the latest version into the canonical dir (all linked
    /// agents see the update immediately), and reports per-skill success/failure
    /// counts. Locally-sourced skills are skipped.
    ///
    /// # Scope resolution
    ///
    /// [`UpdateRequest::scope`] is [`Scope::Auto`] by default: project scope if the
    /// project has skills or a lockfile, otherwise global.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{Manager, UpdateRequest};
    ///
    /// let tmp = tempfile::TempDir::new().unwrap();
    /// let manager = Manager::builder()
    ///     .home(tmp.path().join("home"))
    ///     .cwd(tmp.path().join("project"))
    ///     .build();
    ///
    /// // No lockfile in the scratch dir, so nothing to update.
    /// let outcome = manager.update(&UpdateRequest::default())?;
    /// assert_eq!(outcome.updated, 0);
    /// # Ok::<(), agents_skills::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`SkillsError::Message`] when a recorded source fails to re-parse. Per-skill
    /// clone/install failures are captured in [`UpdateOutcome::failures`] rather than
    /// returned as errors.
    pub fn update(&self, req: &UpdateRequest) -> Result<UpdateOutcome> {
        let global = resolve_scope(req, &self.env);
        let lock_path = lock_path(&self.env, global);
        let lock = read_local_lock(&lock_path);
        // Disabled skills are parked outside the canonical dir; skip them so they stay disabled.
        let disabled_names: HashSet<String> = scan_disabled(&self.env, global)
            .into_iter()
            .map(|n| sanitize_name(&n))
            .collect();
        let skills: Vec<(String, LockEntry)> = lock
            .skills
            .iter()
            .filter(|(name, entry)| {
                matches_skill(name, &req.skills)
                    && entry.source_type != "local"
                    && !disabled_names.contains(&sanitize_name(name))
            })
            .map(|(n, e)| (n.clone(), e.clone()))
            .collect();

        if skills.is_empty() {
            return Ok(UpdateOutcome {
                global,
                ..Default::default()
            });
        }

        // Group by source (same source is cloned only once).
        let mut by_source: BTreeMap<String, Vec<(String, LockEntry)>> = BTreeMap::new();
        for (name, entry) in skills {
            by_source
                .entry(entry.source.clone())
                .or_default()
                .push((name, entry));
        }

        let mut outcome = UpdateOutcome {
            global,
            ..Default::default()
        };
        for (source, items) in &by_source {
            let first = &items[0].1;
            let clone_url = first.source_url.clone().unwrap_or_else(|| source.clone());
            let parsed = parse_source(&clone_url)?;

            // Fetch per source type (archive for github/gitlab/download, clone for git).
            let fetched = match fetch_source(&parsed) {
                Ok(v) => v,
                Err(e) => {
                    for (name, _) in items {
                        outcome.failures.push(format!("{name}: {e}"));
                        outcome.failed += 1;
                    }
                    continue;
                }
            };
            let discovered =
                discover_skills(&fetched.1, parsed.subpath.as_deref(), true).unwrap_or_default();

            for (name, entry) in items {
                let target = find_skill(&discovered, name, entry.skill_path.as_deref());
                let Some(skill) = target else {
                    outcome
                        .failures
                        .push(format!("Skill '{name}' not found in {source}"));
                    outcome.failed += 1;
                    continue;
                };
                let r = install_skill(skill, global, &self.env);
                if r.success {
                    outcome.updated += 1;
                    outcome.updated_names.push(name.clone());
                } else {
                    outcome.failed += 1;
                    outcome.failures.push(format!(
                        "{name}: {}",
                        r.error.unwrap_or_else(|| "install failed".to_string())
                    ));
                }
            }
        }

        Ok(outcome)
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
    /// Affects global skills (`~/.agents/skills`), the global lockfile, and per-agent
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
    /// Affects project-scope installs (`.agents/skills`), the project lockfile, and
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

// ============================ Request types (clap-free) ============================

/// Request for [`Manager::add`].
///
/// The struct is `Default + Clone`; use [`AddRequest::new`] for the common
/// "install everything from a source" case and struct-update syntax
/// (`..Default::default()`) to override just the fields you need.
///
/// See the crate-level [source formats](crate#source-formats) table for the accepted
/// `source` strings.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    /// Source string (local path, GitHub `owner/repo`, git URL, or download URL).
    pub source: String,
    /// Install globally (user-level, `~/.agents/skills`) instead of project-level.
    pub global: bool,
    /// `"*"` or specific skill names; empty = all discovered skills.
    pub skills: Vec<String>,
    /// List available skills without installing anything.
    pub list_only: bool,
}

impl AddRequest {
    /// Create a request that installs all skills from `source` with default options.
    ///
    /// All other fields default: project scope, all skills.
    ///
    /// # Examples
    ///
    /// ```
    /// use agents_skills::{AddRequest, Manager};
    ///
    /// let req = AddRequest::new("anthropics/skills");
    /// assert_eq!(req.source, "anthropics/skills");
    /// assert!(req.skills.is_empty()); // all discovered skills
    /// # let _ = Manager::new();
    /// ```
    pub fn new(source: impl Into<String>) -> Self {
        AddRequest {
            source: source.into(),
            ..Default::default()
        }
    }
}

/// Request for [`Manager::list`].
///
/// `Default` lists project-scope skills across all agents.
#[derive(Debug, Clone, Default)]
pub struct ListRequest {
    /// List global skills instead of project skills.
    pub global: bool,
    /// Filter by agent names; empty = all agents.
    pub agents: Vec<String>,
}

/// Request for [`Manager::remove`].
///
/// `Default` is a no-op that only reports installed names — set `skills` or `all` to
/// actually remove anything.
#[derive(Debug, Clone, Default)]
pub struct RemoveRequest {
    /// Skill names to remove (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Remove global skills instead of project skills.
    pub global: bool,
    /// Remove all installed skills.
    pub all: bool,
}

/// Request for [`Manager::agent`] — one entry point mirroring the `agent` CLI command.
///
/// `Default` links the auto-detected installed agents at project scope.
#[derive(Debug, Clone, Default)]
pub struct AgentRequest {
    /// `"*"` or specific agent names; empty = auto-detect installed agents.
    pub agents: Vec<String>,
    /// Link global skills dirs instead of project ones.
    pub global: bool,
    /// Unlink (disconnect) the agents' skills dirs instead of linking them.
    pub unlink: bool,
    /// Move existing skills into the canonical dir when linking (also pulls
    /// skills parked in the backup slot of an already linked agent).
    pub migrate: bool,
}

/// Installation scope for [`Manager::update`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    /// Auto-detect: project scope if the project has skills or a lockfile, otherwise
    /// global.
    #[default]
    Auto,
    /// Force global scope.
    Global,
    /// Force project scope.
    Project,
}

/// Request for [`Manager::update`].
///
/// `Default` updates all skills with auto-detected scope.
#[derive(Debug, Clone, Default)]
pub struct UpdateRequest {
    /// Filter by skill names; empty = all.
    pub skills: Vec<String>,
    /// Installation scope ([`Scope::Auto`] by default).
    pub scope: Scope,
}

/// Request for [`Manager::disable`].
///
/// `Default` is a no-op that only reports enabled names — set `skills` or `all` to
/// actually disable anything.
#[derive(Debug, Clone, Default)]
pub struct DisableRequest {
    /// Skill names to disable (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Disable global skills instead of project skills.
    pub global: bool,
    /// Disable all currently enabled skills.
    pub all: bool,
}

/// Request for [`Manager::enable`].
///
/// `Default` is a no-op that only reports disabled names — set `skills` or `all` to
/// actually enable anything.
#[derive(Debug, Clone, Default)]
pub struct EnableRequest {
    /// Skill names to enable (the CLI merges positional args and `--skill` here).
    pub skills: Vec<String>,
    /// Enable global skills instead of project skills.
    pub global: bool,
    /// Enable all currently disabled skills.
    pub all: bool,
}

// ============================ Outcome types ============================

/// Result of [`Manager::add`].
///
/// Carries the full picture of an add operation: what was discovered, what was
/// selected, and which skills were installed into the canonical dir.
#[derive(Debug)]
pub struct AddOutcome {
    /// The parsed source.
    pub source: Source,
    /// All discovered skills.
    pub skills: Vec<Skill>,
    /// Selected skills (empty when `list_only`).
    pub selected: Vec<Skill>,
    /// Successfully installed skills.
    pub installed: Vec<InstallSuccess>,
    /// Failed installations.
    pub failed: Vec<InstallFailure>,
    /// Whether this was a `--list` request.
    pub list_only: bool,
}

/// A single successful install (one skill, into the canonical dir).
#[derive(Debug)]
pub struct InstallSuccess {
    /// Skill name.
    pub name: String,
    /// Canonical directory.
    pub canonical_path: PathBuf,
}

/// A single failed install.
#[derive(Debug)]
pub struct InstallFailure {
    /// Skill name.
    pub skill: String,
    /// Error message.
    pub error: String,
}

/// Result of linking (or unlinking) one agent's skills dir relative to the
/// canonical dir.
#[derive(Debug)]
pub struct AgentLinkResult {
    /// Agent identifier (as used on the CLI).
    pub agent: String,
    /// Agent display name.
    pub display: String,
    /// Link/unlink outcome details.
    pub outcome: LinkOutcome,
}

/// Link status of one agent (used by `agent --status`).
#[derive(Debug)]
pub struct AgentStatus {
    /// Agent identifier (as used on the CLI).
    pub name: String,
    /// Agent display name.
    pub display: String,
    /// Whether the agent's skills dir is linked to the canonical dir.
    pub linked: bool,
    /// Whether the agent natively uses the canonical dir (no link involved).
    pub canonical: bool,
    /// Skills inside the agent's own skills dir: real subdirs and dir-targeting
    /// symlinks — the same classification link and migrate use. Only populated
    /// for unlinked, non-canonical agents; empty for linked/canonical agents
    /// (they share the canonical dir, shown by `list`).
    pub internal_skills: Vec<String>,
    /// Non-skill entries (files, symlinks to non-directories) inside the agent's
    /// own skills dir. Same population rules as [`AgentStatus::internal_skills`].
    pub internal_others: Vec<String>,
    /// Backup slot with parked content waiting for unlink to restore, if any.
    pub pending_backup: Option<BackupStatus>,
}

/// A pending backup slot (used by `agent --status`).
#[derive(Debug)]
pub struct BackupStatus {
    /// Backup slot directory (`.agents/backup-skills/<agent>`).
    pub path: PathBuf,
    /// Names of the entries parked in the slot.
    pub items: Vec<String>,
}

/// Result of [`Manager::agent`].
#[derive(Debug)]
pub struct AgentOutcome {
    /// Whether the links used global scope.
    pub global: bool,
    /// Per-agent link results.
    pub results: Vec<AgentLinkResult>,
}

/// A listed skill enriched with lock metadata (serialized by `list --json`).
///
/// Fields are serialized in camelCase, so `source_url` becomes `"sourceUrl"` — the
/// exact JSON shape emitted by the CLI's `list --json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedSkill {
    /// Skill name.
    pub name: String,
    /// Canonical directory path.
    pub path: PathBuf,
    /// `"project"` or `"global"`.
    pub scope: String,
    /// Agent display names this skill is linked to.
    pub agents: Vec<String>,
    /// Source identifier from the lock.
    pub source: Option<String>,
    /// Resolved source URL.
    pub source_url: Option<String>,
    /// Source type (e.g. `"github"`, `"local"`).
    pub source_type: Option<String>,
    /// Whether the skill is enabled (`true`) or parked in `disabled-skills` (`false`).
    pub enabled: bool,
}

/// Result of [`Manager::remove`].
#[derive(Debug)]
pub struct RemoveOutcome {
    /// Installed names scanned (used by the no-args hint).
    pub installed: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually removed.
    pub removed: Vec<String>,
}

/// Result of [`Manager::update`].
#[derive(Debug, Default)]
pub struct UpdateOutcome {
    /// Whether the update used global scope.
    pub global: bool,
    /// Number of successful updates (one count per skill).
    pub updated: usize,
    /// Number of failed updates.
    pub failed: usize,
    /// Names of skills that were updated.
    pub updated_names: Vec<String>,
    /// Human-readable failure messages.
    pub failures: Vec<String>,
}

/// Result of [`Manager::disable`].
#[derive(Debug)]
pub struct DisableOutcome {
    /// Currently enabled names (used by the no-args hint).
    pub installed: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually disabled.
    pub disabled: Vec<String>,
    /// Names that were already disabled (idempotent no-op).
    pub already: Vec<String>,
    /// Requested names that matched neither enabled nor disabled skills.
    pub missing: Vec<String>,
}

/// Result of [`Manager::enable`].
#[derive(Debug)]
pub struct EnableOutcome {
    /// Currently disabled names (used by the no-args hint).
    pub disabled: Vec<String>,
    /// Requested names (used by the no-match hint).
    pub requested: Vec<String>,
    /// Names actually enabled.
    pub enabled: Vec<String>,
    /// Names that were already enabled (idempotent no-op).
    pub already: Vec<String>,
    /// Requested names that matched neither enabled nor disabled skills.
    pub missing: Vec<String>,
}

// ============================ Private helpers ============================

/// Resolve agent selection: `"*"` → all; names → validated; empty → auto-detect + universal.
fn resolve_target_agents(names: &[String], env: &Env) -> Result<Vec<&'static Agent>> {
    if names.iter().any(|a| a == "*") {
        return Ok(AGENTS.iter().collect());
    }
    if !names.is_empty() {
        let mut agents = Vec::new();
        let mut invalid = Vec::new();
        for name in names {
            match get_agent(name) {
                Some(a) => agents.push(a),
                None => invalid.push(name.clone()),
            }
        }
        if !invalid.is_empty() {
            return Err(SkillsError::InvalidAgents(invalid.join(", ")));
        }
        return Ok(agents);
    }
    let installed = detect_installed_agents(env);
    Ok(ensure_universal_agents(installed))
}

/// Resolve requested names against available name sets, matching case-insensitively on
/// sanitized names. `sources` are consulted in order and the first available original
/// name wins — put higher-priority sources (e.g. lock keys) first.
fn resolve_names(requested: &[String], sources: &[&[String]]) -> Vec<String> {
    let mut identity: HashMap<String, String> = HashMap::new();
    for source in sources {
        for folder in *source {
            identity
                .entry(sanitize_name(folder))
                .or_insert_with(|| folder.clone());
        }
    }
    let mut matched = HashSet::new();
    for name in requested {
        if let Some(hit) = identity.get(&sanitize_name(name)) {
            matched.insert(hit.clone());
        }
    }
    let mut v: Vec<String> = matched.into_iter().collect();
    v.sort();
    v
}

/// Core enable/disable: resolve requested names, classify idempotent/missing, and move
/// dirs. Returns `(selected, already, missing)` where `selected` were actually moved.
///
/// `from_set` holds names in the current state (source of the move); `target_set` holds
/// names in the target state (to detect idempotent no-ops).
fn set_enabled_state(
    requested: &[String],
    from_set: &[String],
    target_set: &[String],
    global: bool,
    to_enabled: bool,
    env: &Env,
) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let selected = resolve_names(requested, &[from_set]);

    let mut already: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in requested {
        if selected
            .iter()
            .any(|s| sanitize_name(s) == sanitize_name(name))
        {
            continue;
        }
        if target_set
            .iter()
            .any(|d| sanitize_name(d) == sanitize_name(name))
        {
            already.push(name.clone());
        } else {
            missing.push(name.clone());
        }
    }
    already.sort();
    already.dedup();
    missing.sort();
    missing.dedup();

    for name in &selected {
        move_skill(name, global, to_enabled, env)?;
    }

    Ok((selected, already, missing))
}

/// Combine `--skill` args with the source's `@skill` filter into one selection list.
fn skill_filters(skills: &[String], skill_filter: Option<&str>) -> Vec<String> {
    let mut filters = skills.to_vec();
    if let Some(sf) = skill_filter {
        filters.push(sf.to_string());
    }
    filters
}

/// Match a discovered skill by sanitized name or skillPath directory name.
fn find_skill<'a>(
    discovered: &'a [Skill],
    name: &str,
    skill_path: Option<&str>,
) -> Option<&'a Skill> {
    let sanitized = sanitize_name(name);
    // Prefer matching by (sanitized) name.
    if let Some(s) = discovered
        .iter()
        .find(|s| sanitize_name(&s.name) == sanitized)
    {
        return Some(s);
    }
    // Match by skillPath (directory name).
    if let Some(sp) = skill_path
        && let Some(dn) = sp.split('/').rfind(|p| !p.is_empty())
        && let Some(s) = discovered.iter().find(|s| {
            s.dir
                .file_name()
                .map(|f| f.to_string_lossy() == dn)
                .unwrap_or(false)
        })
    {
        return Some(s);
    }
    discovered.first().filter(|_| discovered.len() == 1)
}

/// Whether a skill name matches a case-insensitive filter (empty filter matches all).
fn matches_skill(name: &str, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    filter.iter().any(|f| f.to_lowercase() == lower)
}

/// Resolve skill names to remove: match by sanitized name, lock keys take priority.
fn resolve_to_remove(
    requested: &[String],
    installed: &[String],
    disabled: &[String],
    lock_keys: &[String],
) -> Vec<String> {
    resolve_names(requested, &[lock_keys, installed, disabled])
}

fn lock_path(env: &Env, global: bool) -> PathBuf {
    if global {
        global_lock_path(&env.home)
    } else {
        local_lock_path(&env.cwd)
    }
}

fn write_lock(
    parsed: &Source,
    selected: &[Skill],
    successful: &[InstallSuccess],
    global: bool,
    env: &Env,
) -> Result<()> {
    let lock_path = lock_path(env, global);
    let mut lock = read_local_lock(&lock_path);
    lock.version = 1;

    let successful_names: HashSet<&str> = successful.iter().map(|s| s.name.as_str()).collect();
    for skill in selected {
        if !successful_names.contains(skill.name.as_str()) {
            continue;
        }
        let hash = compute_folder_hash(&skill.dir).unwrap_or_default();
        let (source, source_type, source_url, ref_, skill_path) = lock_fields(parsed);
        let mut entry = LockEntry::new(&source, &source_type, hash);
        entry.source_url = source_url;
        entry.r#ref = ref_;
        entry.skill_path = skill_path;
        lock.skills.insert(sanitize_name(&skill.name), entry);
    }
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_local_lock(&lock, &lock_path)
}

fn resolve_scope(req: &UpdateRequest, env: &Env) -> bool {
    match req.scope {
        Scope::Global => true,
        Scope::Project => false,
        Scope::Auto => !has_project_skills(env),
    }
}

fn has_project_skills(env: &Env) -> bool {
    if local_lock_path(&env.cwd).exists() {
        return true;
    }
    env.cwd.join(".agents/skills").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_utils::{env_at, write_and_parse_skill};

    fn skills_with_dirs(pairs: &[(&str, &str)]) -> Vec<Skill> {
        pairs
            .iter()
            .map(|(dir, name)| {
                let mut s = write_and_parse_skill(std::path::Path::new(dir), name);
                // write_and_parse_skill derives dir from the SKILL.md path; use the skill dir.
                s.dir = std::path::PathBuf::from(dir);
                s
            })
            .collect()
    }

    #[test]
    fn find_skill_prefers_name_then_skill_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let a = tmp.path().join("dir-a");
        let b = tmp.path().join("dir-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let skills = skills_with_dirs(&[
            (a.to_str().unwrap(), "alpha"),
            (b.to_str().unwrap(), "beta"),
        ]);

        // By (sanitized) name.
        assert_eq!(find_skill(&skills, "Alpha", None).unwrap().name, "alpha");
        // By skillPath directory name.
        assert_eq!(
            find_skill(&skills, "missing", Some("x/dir-b"))
                .unwrap()
                .name,
            "beta"
        );
        // Ambiguous without a match.
        assert!(find_skill(&skills, "missing", None).is_none());
    }

    #[test]
    fn matches_skill_filters_case_insensitively() {
        let filter = vec!["PDF".to_string()];
        assert!(matches_skill("pdf", &filter));
        assert!(!matches_skill("git", &filter));
        assert!(matches_skill("anything", &[]));
    }

    #[test]
    fn skill_filters_merges_args_and_at_filter() {
        assert_eq!(skill_filters(&[], Some("pdf")), vec!["pdf"]);
        assert_eq!(skill_filters(&["x".to_string()], None), vec!["x"]);
        assert_eq!(
            skill_filters(&["x".to_string()], Some("pdf")),
            vec!["x", "pdf"]
        );
        assert!(skill_filters(&[], None).is_empty());
    }

    #[test]
    fn resolve_to_remove_prefers_lock_keys() {
        let installed = vec!["PDF".to_string()];
        let lock_keys = vec!["pdf".to_string()];
        let requested = vec!["pdf".to_string(), "unknown".to_string()];

        // Lock keys take priority: "pdf" (not the on-disk "PDF" casing).
        assert_eq!(
            resolve_to_remove(&requested, &installed, &[], &lock_keys),
            vec!["pdf"]
        );
    }

    #[test]
    fn resolve_to_remove_matches_disabled_without_lock() {
        let installed = Vec::new();
        let disabled = vec!["legacy".to_string()];
        let lock_keys = Vec::new();
        let requested = vec!["legacy".to_string()];

        // A disabled skill with no lockfile entry is still resolvable for removal.
        assert_eq!(
            resolve_to_remove(&requested, &installed, &disabled, &lock_keys),
            vec!["legacy"]
        );
    }

    #[test]
    fn resolve_target_agents_validates_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        assert!(resolve_target_agents(&["claude-code".to_string()], &env).is_ok());
        assert!(matches!(
            resolve_target_agents(&["nope".to_string()], &env),
            Err(SkillsError::InvalidAgents(_))
        ));
    }
}
