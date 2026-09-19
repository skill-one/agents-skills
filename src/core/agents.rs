//! agent → skills directory mapping (data-driven table) + directory resolution.
//!
//! The table itself lives in [`agents.jsonl`](agents.jsonl) (one JSON object per line,
//! embedded into the binary at compile time): adding or changing an agent is a one-line
//! edit, no Rust changes required. Resolution is fully declarative — [`PathSpec`] probes
//! are interpreted against the injectable [`Env`], making unit tests easy (build a temp
//! dir, no touching the real environment).
//!
//! Skills live in exactly one place — the canonical dir
//! ([`canonical_skills_dir`], `~/.agents/skills`). Each agent reads its own
//! directory ([`agent_skills_dir`]), which `agent --link` points at the canonical
//! dir with a symlink.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::Deserialize;

use crate::core::path_util::normalize_lexical;

/// The canonical skills dir, relative to the home directory.
pub const UNIVERSAL_SKILLS_DIR: &str = ".agents/skills";

/// The sibling dir where disabled skills are parked (never symlinked to agents).
pub const DISABLED_SKILLS_DIR: &str = ".agents/disabled-skills";

/// Embedded agent table: one JSON object per agent line (blank lines and `#` comments
/// allowed). Schema documented in `docs/DEVELOPER.md`.
static AGENT_TABLE_JSONL: &str = include_str!("agents.jsonl");

/// Context for agent detection/dir resolution (owns data, easy to inject in tests).
pub struct Env {
    /// Home directory (`~`).
    pub home: PathBuf,
    /// Config directory (`$XDG_CONFIG_HOME` or `~/.config`).
    pub config: PathBuf,
    /// Current working directory.
    pub cwd: PathBuf,
    /// Environment variables injectable in tests (reads the real env when None).
    vars: Option<HashMap<String, String>>,
    /// Whether detection may probe well-known system locations outside
    /// `home`/`config`/`cwd` (e.g. `/Applications/ZCode.app`). Tests and
    /// sandboxes turn this off so detection stays hermetic.
    probe_system_dirs: bool,
}

impl Env {
    /// Construct an environment context from explicit paths.
    pub fn new(home: impl AsRef<Path>, config: impl AsRef<Path>, cwd: impl AsRef<Path>) -> Self {
        Env {
            home: home.as_ref().to_path_buf(),
            config: config.as_ref().to_path_buf(),
            cwd: cwd.as_ref().to_path_buf(),
            vars: None,
            probe_system_dirs: true,
        }
    }

    /// Read an environment variable (injectable override in tests).
    pub fn var(&self, key: &str) -> Option<String> {
        match &self.vars {
            Some(map) => map.get(key).cloned(),
            None => std::env::var(key).ok(),
        }
    }

    /// Inject environment variable overrides (for library users and tests).
    pub fn set_vars(&mut self, vars: HashMap<String, String>) {
        self.vars = Some(vars);
    }

    /// Toggle probing of well-known system locations outside `home`/`config`/`cwd`.
    ///
    /// Tests and sandboxes pass `false` so agent detection never consults the
    /// real machine (e.g. `/Applications`), keeping results hermetic.
    pub fn set_probe_system_dirs(&mut self, probe: bool) {
        self.probe_system_dirs = probe;
    }
}

/// config dir: `$XDG_CONFIG_HOME || ~/.config`.
pub fn config_home() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => home().join(".config"),
    }
}

/// Resolve the user's home directory.
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// `$VAR || home/<default>` (with an optional sub-path) — agent homes like Claude's.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvHomeSpec {
    /// Environment variable consulted first (e.g. `CLAUDE_CONFIG_DIR`).
    pub var: String,
    /// Home-relative fallback when the var is unset or blank (e.g. `.claude`).
    pub default: String,
    /// Optional sub-path joined after the base (e.g. `skills`).
    #[serde(default)]
    pub path: Option<String>,
}

/// `$VAR/<path>` — a var-derived location (e.g. Zed's `%APPDATA%/Zed`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvVarSpec {
    /// Environment variable (e.g. `APPDATA`).
    pub var: String,
    /// Optional sub-path joined after the var value.
    #[serde(default)]
    pub path: Option<String>,
}

/// One declarative path probe: exactly one of the keys below may appear.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PathSpec {
    /// Exists when `home/<home>` exists.
    Home {
        /// Path relative to the home directory.
        home: String,
    },
    /// Exists when `config/<config>` exists.
    Config {
        /// Path relative to the config directory.
        config: String,
    },
    /// Exists when `cwd/<cwd>` exists.
    Cwd {
        /// Path relative to the working directory.
        cwd: String,
    },
    /// `$VAR || home/<default>`, optionally joined with a sub-path.
    EnvHome {
        /// The env-home spec.
        env_home: EnvHomeSpec,
    },
    /// `$VAR/<path>`; unmatched when the var is unset.
    EnvVar {
        /// The env-var spec.
        env_var: EnvVarSpec,
    },
    /// Absolute system location; only probed when system probing is enabled.
    System {
        /// Absolute path (e.g. `/Applications/ZCode.app`).
        system: String,
    },
}

impl PathSpec {
    /// Resolve against `env`; `None` = not applicable (e.g. var unset, probing off).
    fn resolve(&self, env: &Env) -> Option<PathBuf> {
        match self {
            PathSpec::Home { home } => Some(env.home.join(home)),
            PathSpec::Config { config } => Some(env.config.join(config)),
            PathSpec::Cwd { cwd } => Some(env.cwd.join(cwd)),
            PathSpec::EnvHome { env_home } => Some(env_home.resolve(env)),
            PathSpec::EnvVar { env_var } => env_var.resolve(env),
            PathSpec::System { system } => env.probe_system_dirs.then(|| PathBuf::from(system)),
        }
    }
}

impl EnvHomeSpec {
    fn resolve(&self, env: &Env) -> PathBuf {
        let base = match env.var(&self.var) {
            Some(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
            _ => env.home.join(&self.default),
        };
        match &self.path {
            Some(p) => base.join(p),
            None => base,
        }
    }
}

impl EnvVarSpec {
    fn resolve(&self, env: &Env) -> Option<PathBuf> {
        let base = PathBuf::from(env.var(&self.var)?);
        Some(match &self.path {
            Some(p) => base.join(p),
            None => base,
        })
    }
}

/// An agent's directory config (one line of the embedded JSONL table).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// Agent identifier (used on the CLI).
    pub name: String,
    /// Human-readable display name.
    pub display: String,
    /// Skills directory the agent reads.
    pub global: PathSpec,
    /// Install detection rules (any match = installed; empty = never detected).
    #[serde(default)]
    pub detect: Vec<PathSpec>,
}

/// Parse the JSONL table; invalid input is a build bug, so it panics with the line number.
fn parse_agent_table(jsonl: &str) -> Vec<Agent> {
    let mut agents: Vec<Agent> = Vec::new();
    for (idx, line) in jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let agent: Agent = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("agents.jsonl line {}: {e}", idx + 1));
        if let Some(prev) = agents.iter().position(|a| a.name == agent.name) {
            panic!(
                "agents.jsonl line {}: duplicate agent '{}' (first defined on line {})",
                idx + 1,
                agent.name,
                prev + 1
            );
        }
        agents.push(agent);
    }
    assert!(
        !agents.is_empty(),
        "agents.jsonl must define at least one agent"
    );
    agents
}

/// Agent table, parsed once from the embedded JSONL (leaked to get `&'static` entries).
pub static AGENTS: LazyLock<&'static [Agent]> = LazyLock::new(|| {
    Box::leak(parse_agent_table(AGENT_TABLE_JSONL).into_boxed_slice()) as &'static [Agent]
});

/// Look up an agent by name.
pub fn get_agent(name: &str) -> Option<&'static Agent> {
    let agents: &'static [Agent] = *AGENTS;
    agents.iter().find(|a| a.name == name)
}

/// Whether an agent natively reads the canonical dir, so no symlink is needed:
/// its resolved skills dir is `~/.agents/skills` itself.
pub fn is_native(agent: &Agent, env: &Env) -> bool {
    agent
        .global
        .resolve(env)
        .is_some_and(|dir| normalize_lexical(&dir) == normalize_lexical(&canonical_skills_dir(env)))
}

/// Canonical skills dir: `~/.agents/skills`.
pub fn canonical_skills_dir(env: &Env) -> PathBuf {
    env.home.join(UNIVERSAL_SKILLS_DIR)
}

/// Disabled skills dir: `~/.agents/disabled-skills`.
///
/// Disabled skills are moved here, out of the canonical dir, so no agent (linked
/// or native) sees them. Enabling moves them back.
pub fn disabled_skills_dir(env: &Env) -> PathBuf {
    env.home.join(DISABLED_SKILLS_DIR)
}

/// An agent's own skills dir.
///
/// `None` only when the location cannot be resolved (e.g. an `env_var`-based dir
/// whose variable is unset). For scope-native agents the returned dir is the
/// canonical dir itself; check [`is_native`] before treating it as a linkable
/// location.
pub fn agent_skills_dir(agent: &Agent, env: &Env) -> Option<PathBuf> {
    agent.global.resolve(env)
}

/// Determine whether an agent is installed: any detection rule resolves to an existing path.
pub fn is_installed(agent: &Agent, env: &Env) -> bool {
    agent
        .detect
        .iter()
        .any(|spec| spec.resolve(env).is_some_and(|p| p.exists()))
}

/// Detect currently installed agents.
pub fn detect_installed_agents(env: &Env) -> Vec<&'static Agent> {
    let agents: &'static [Agent] = *AGENTS;
    agents.iter().filter(|a| is_installed(a, env)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_utils::env_at;

    #[test]
    fn agent_table_has_key_agents() {
        assert!(get_agent("claude-code").is_some());
        assert!(get_agent("codex").is_some());
        assert!(get_agent("universal").is_some());
        assert!(get_agent("amp").is_some());
        assert!(get_agent("nope").is_none());
    }

    #[test]
    fn skills_dir_resolution() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);

        // home base
        let cline = get_agent("cline").unwrap();
        assert_eq!(
            agent_skills_dir(cline, &env).unwrap(),
            tmp.path().join(".agents/skills")
        );
        // config base
        let amp = get_agent("amp").unwrap();
        assert_eq!(
            agent_skills_dir(amp, &env).unwrap(),
            tmp.path().join("config/agents/skills")
        );
        // vendor-specific home sub-path
        let claude = get_agent("claude-code").unwrap();
        assert_eq!(
            agent_skills_dir(claude, &env).unwrap(),
            tmp.path().join(".claude/skills")
        );
    }

    #[test]
    fn is_native_only_for_the_canonical_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        // Its skills dir is ~/.agents/skills itself.
        assert!(is_native(get_agent("cline").unwrap(), &env));
        // Vendor-specific dirs must be linked.
        assert!(!is_native(get_agent("claude-code").unwrap(), &env));
        assert!(!is_native(get_agent("antigravity").unwrap(), &env));
        assert!(!is_native(get_agent("codex").unwrap(), &env));
    }

    #[test]
    fn is_native_false_when_env_var_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        // promptscript's dir is env-var-based: unresolved → not native.
        let promptscript = get_agent("promptscript").unwrap();
        assert!(!is_native(promptscript, &env));
        assert_eq!(agent_skills_dir(promptscript, &env), None);
    }

    #[test]
    fn canonical_and_disabled_dirs_live_under_home() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        assert_eq!(
            canonical_skills_dir(&env),
            tmp.path().join(".agents/skills")
        );
        assert_eq!(
            disabled_skills_dir(&env),
            tmp.path().join(".agents/disabled-skills")
        );
    }

    #[test]
    fn detect_home_config_cwd() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);

        std::fs::create_dir_all(tmp.path().join(".cline")).unwrap();
        assert!(is_installed(get_agent("cline").unwrap(), &env));

        std::fs::create_dir_all(tmp.path().join("config/amp")).unwrap();
        assert!(is_installed(get_agent("amp").unwrap(), &env));

        std::fs::create_dir_all(tmp.path().join(".replit")).unwrap();
        assert!(is_installed(get_agent("replit").unwrap(), &env));
    }

    #[test]
    fn detect_home_or_cwd() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codebuddy")).unwrap();
        assert!(is_installed(get_agent("codebuddy").unwrap(), &env));
    }

    #[test]
    fn agent_without_detect_rules_is_never_installed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = env_at(&tmp);
        assert!(!is_installed(get_agent("universal").unwrap(), &env));
    }

    // Declarative detection semantics (the rules that used to live in special_detect).

    #[test]
    fn env_home_prefers_var_over_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut env = env_at(&tmp);
        let claude = get_agent("claude-code").unwrap();

        // Default home: ~/.claude.
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        assert!(is_installed(claude, &env));

        // Var override: $CLAUDE_CONFIG_DIR wins, default no longer consulted.
        let mut vars = HashMap::new();
        vars.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            tmp.path().join("cc").display().to_string(),
        );
        env.set_vars(vars);
        assert!(!is_installed(claude, &env));
        std::fs::create_dir_all(tmp.path().join("cc")).unwrap();
        assert!(is_installed(claude, &env));
    }

    #[test]
    fn env_var_unset_never_matches() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut env = env_at(&tmp);
        env.set_vars(HashMap::new()); // no APPDATA
        let zed = get_agent("zed").unwrap();
        assert!(!is_installed(zed, &env)); // config/zed also missing

        let mut vars = HashMap::new();
        vars.insert(
            "APPDATA".to_string(),
            tmp.path().join("appdata").display().to_string(),
        );
        env.set_vars(vars);
        assert!(!is_installed(zed, &env)); // set but path missing
        std::fs::create_dir_all(tmp.path().join("appdata/Zed")).unwrap();
        assert!(is_installed(zed, &env));
    }

    #[test]
    fn system_probe_respects_probe_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut env = env_at(&tmp);
        env.set_vars(HashMap::new());
        env.set_probe_system_dirs(true);
        let marker = tmp.path().join("sys-marker"); // absolute, outside home/config/cwd
        std::fs::create_dir_all(&marker).unwrap();

        let spec = PathSpec::System {
            system: marker.display().to_string(),
        };
        assert!(spec.resolve(&env).is_some());
        env.set_probe_system_dirs(false);
        assert!(spec.resolve(&env).is_none());
    }

    #[test]
    fn agent_table_parses_comments_and_blank_lines() {
        let table = parse_agent_table(
            "# header comment\n\n{\"name\":\"x\",\"display\":\"X\",\"global\":{\"home\":\".x/skills\"},\"detect\":[{\"home\":\".x\"}]}\n\n",
        );
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].name, "x");
    }

    #[test]
    #[should_panic(expected = "duplicate agent 'x'")]
    fn agent_table_rejects_duplicate_names() {
        let one = r#"{"name":"x","display":"X","global":{"home":".x"},"detect":[]}"#;
        let two = r#"{"name":"x","display":"Y","global":{"home":".y"},"detect":[]}"#;
        parse_agent_table(&format!("{one}\n{two}\n"));
    }

    #[test]
    #[should_panic(expected = "agents.jsonl line 2:")]
    fn agent_table_reports_offending_line() {
        parse_agent_table(
            "{\"name\":\"x\",\"display\":\"X\",\"global\":{\"home\":\".x\"},\"detect\":[]}\nnot json\n",
        );
    }

    #[test]
    fn agent_table_row_count() {
        let agents: &'static [Agent] = *AGENTS;
        assert_eq!(agents.len(), 84);
    }
}
