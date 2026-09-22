//! High-level `Manager` facade: one-stop add/list/remove over an injectable [`Env`].
//!
//! The manager is pure data: it returns structured outcomes and never prints or exits;
//! the CLI layer (src/commands) is responsible for rendering.

use std::path::{Path, PathBuf};

use crate::core::agents::{
    AGENTS, Env, canonical_skills_dir, config_home, disabled_skills_dir, home, is_installed,
    is_native,
};
use crate::core::discover::{Skill, discover_skills, filter_skills};
use crate::core::fetch::fetch_source;
use crate::core::github::{fetch_skill_via_api, fetch_subdir_via_api};
use crate::core::install::{
    install_skill, list_disabled_skills, list_installed_skills, remove_skill, scan_disabled,
    scan_installed,
};
use crate::core::link::{is_agent_linked, link_agent, private_content, unlink_agent};
use crate::core::source::{Source, SourceType, parse_source};
use crate::error::{Result, SkillsError};

use crate::manager::select::{
    resolve_target_agents, resolve_to_remove, set_enabled_state, skill_filters,
};
pub use crate::manager::types::{
    AddOutcome, AddRequest, AgentLinkResult, AgentOutcome, AgentRequest, AgentStatus,
    DisableOutcome, DisableRequest, EnableOutcome, EnableRequest, InstallFailure, InstallSuccess,
    ListedSkill, RemoveOutcome, RemoveRequest,
};
mod select;
#[cfg(test)]
mod tests;
mod types;

/// Require `subpath` to exist inside the fetched `root`.
///
/// The archive path is the one that needs this check: it serves sources the API
/// cannot narrow (GitLab), so a subpath that resolves to nothing there is a user
/// error worth naming.
fn require_subpath(root: &Path, subpath: &str, source: &Source) -> Result<()> {
    if root.join(subpath).exists() {
        return Ok(());
    }
    Err(subpath_error(source, subpath))
}

/// The error for a `subpath` that resolved to nothing.
fn subpath_error(source: &Source, subpath: &str) -> SkillsError {
    SkillsError::msg(format!(
        "Subpath \"{subpath}\" not found in {} — check the path and the ref.",
        source_label(source)
    ))
}

/// The error for a skill name the API could not find.
fn missing_skill_error(parsed: &Source, req: &AddRequest, name: &str) -> SkillsError {
    SkillsError::msg(format!(
        "No skill named \"{name}\" in {}. Run `agents-skills add {} --list` to list the available skills.",
        source_label(parsed),
        req.source
    ))
}

/// A repository URL without its `.git` suffix, for error messages.
fn source_label(source: &Source) -> &str {
    source.url.trim_end_matches(".git")
}

/// A hint appended when a failure looks like a GitHub rate limit.
///
/// The unauthenticated limit is 60 requests/hour per IP, which a handful of
/// narrowed installs can exhaust; a token raises it to 5000.
fn rate_limit_hint(error: &SkillsError) -> &'static str {
    match error {
        SkillsError::Http(e) if matches!(e.as_ref(), ureq::Error::StatusCode(403 | 429)) => {
            " Set GITHUB_TOKEN to raise the API rate limit from 60 to 5000 requests/hour."
        }
        _ => "",
    }
}

/// Whether this request must be served by the GitHub API.
///
/// Both accepted shapes narrow the install — a `subpath`, or `--skill` / `@skill` —
/// so the API can fetch exactly that much. Everything else is served by the
/// whole-repo archive, which the API cannot narrow: a repository-wide install,
/// `--list` (it needs the whole tree to report every skill), and GitLab (no API
/// path is implemented for it).
fn uses_github_api(parsed: &Source, list_only: bool) -> bool {
    parsed.ty == SourceType::Github
        && ((parsed.skill_filter.is_some() && !list_only) || parsed.subpath.is_some())
}

/// Fetch a narrowed request through the GitHub API.
///
/// There is deliberately no fallback to the whole-repo archive. It would silently
/// widen a subpath install to the entire repository — and in the two commonest
/// failures, a mistyped subpath or skill name, it would download everything only to
/// report the very same error.
fn fetch_narrowed(
    parsed: &Source,
    req: &AddRequest,
    include_internal: bool,
) -> Result<(tempfile::TempDir, PathBuf)> {
    let fetched = match (parsed.skill_filter.as_deref(), req.list_only) {
        (Some(name), false) => fetch_skill_via_api(parsed, name, include_internal),
        _ => fetch_subdir_via_api(parsed),
    };

    match fetched {
        Ok(Some(v)) => Ok(v),
        // The API answered: the request simply matched nothing in the repository.
        Ok(None) => Err(match parsed.skill_filter.as_deref() {
            Some(name) => missing_skill_error(parsed, req, name),
            None => subpath_error(parsed, parsed.subpath.as_deref().unwrap_or_default()),
        }),
        Err(e) => Err(SkillsError::msg(format!(
            "GitHub API request failed for {}: {e}.{}",
            source_label(parsed),
            rate_limit_hint(&e)
        ))),
    }
}

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
    /// structured [`AddOutcome`] with discovered, selected, installed, skipped
    /// and failed skills.
    ///
    /// `add` only ever adds: a selected skill whose name is already installed —
    /// enabled *or* disabled — is reported in [`AddOutcome`] `skipped` and left
    /// untouched, so local edits are never silently discarded. Replace an
    /// installed skill with [`Manager::remove`] followed by `add`.
    ///
    /// `add` never links any agent: use [`Manager::agent`] to expose the canonical
    /// dir to an agent afterwards.
    ///
    /// # Fetching
    ///
    /// A request narrowed by a `subpath` or by a skill name (`--skill` / `@skill`)
    /// is served by the GitHub API, which downloads only the matching files — and
    /// which never falls back to a whole-repo archive, so a failure is reported
    /// instead of silently widening the install. Set `GITHUB_TOKEN` to raise its
    /// rate limit (60 → 5000 requests/hour). Everything else is served by the
    /// whole-repo archive, the API being unable to narrow it: a repository-wide
    /// install, `list_only` (it needs the whole tree to report every skill), and
    /// GitLab.
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
    /// - [`SkillsError::Message`] when a narrowed GitHub fetch fails: an unknown
    ///   `subpath` or skill name, or an API that is unavailable (see *Fetching*).
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
            // Two fetch modes, both handing back a repository-root temp dir so that
            // the subpath below means the same thing either way: the GitHub API for
            // narrowed requests, the whole-repo archive for everything it cannot
            // serve. See `uses_github_api` / `fetch_narrowed`.
            let (tmp, root) = if uses_github_api(&parsed, req.list_only) {
                fetch_narrowed(&parsed, req, include_internal)?
            } else {
                fetch_source(&parsed)?
            };
            if let Some(sp) = parsed.subpath.as_deref() {
                require_subpath(&root, sp, &parsed)?;
            }
            skills = discover_skills(&root, parsed.subpath.as_deref(), include_internal)?;
            _temp = Some(tmp);
        }

        if skills.is_empty() {
            let msg = match parsed.subpath.as_deref() {
                Some(sp) => format!(
                    "No skills found under \"{sp}\". A skill needs a SKILL.md with name and description."
                ),
                None => {
                    "No valid skills found. Skills require a SKILL.md with name and description."
                        .to_string()
                }
            };
            return Err(SkillsError::msg(msg));
        }

        // --list: report discovered skills without installing.
        if req.list_only {
            return Ok(AddOutcome {
                source: parsed,
                skills,
                selected: Vec::new(),
                installed: Vec::new(),
                skipped: Vec::new(),
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
        // An already-installed name (enabled or disabled) is skipped, not replaced.
        let mut installed: Vec<InstallSuccess> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut failed: Vec<InstallFailure> = Vec::new();
        for skill in &selected {
            let r = install_skill(skill, &self.env);
            if !r.success {
                failed.push(InstallFailure {
                    skill: skill.name.clone(),
                    error: r.error.unwrap_or_default(),
                });
            } else if r.skipped {
                skipped.push(skill.name.clone());
            } else {
                installed.push(InstallSuccess {
                    name: skill.name.clone(),
                    canonical_path: r.canonical_path,
                });
            }
        }

        Ok(AddOutcome {
            source: parsed,
            skills,
            selected,
            installed,
            skipped,
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
