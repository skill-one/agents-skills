# agents-skills Library Guide

English | [简体中文](LIBRARY.zh-CN.md)

For **library users**: embed the skill-management capabilities into your own
Rust tooling. For CLI usage see the [README](../README.md), for the command
reference see [CLI.md](CLI.md).

## Adding the dependency

```toml
[dependencies]
agents-skills = "0.10"
```

## Quick start

```rust
use agents_skills::{AddRequest, AgentRequest, Manager};

fn main() -> agents_skills::Result<()> {
    let manager = Manager::builder().build(); // equivalent to Manager::new()

    manager.agent(&AgentRequest::default())?;        // link all installed agents
    let outcome = manager.add(&AddRequest::new("anthropics/skills"))?; // install a skill pack
    println!("installed {} skill(s)", outcome.installed.len());

    // agent_status lists each agent's link status; unlinked agents that carry
    // their own content expose their private skills and other files — what
    // linking would adopt into the canonical directory.
    for s in manager.agent_status(false) {
        println!("{}: linked={}", s.name, s.linked);
        if !s.internal_skills.is_empty() {
            println!("  skills: {}", s.internal_skills.join(", "));
        }
        if !s.internal_others.is_empty() {
            println!("  others: {}", s.internal_others.join(", "));
        }
    }
    Ok(())
}
```

## High-level API: [`Manager`]

Every method takes a pure-data request struct and returns a structured result;
all request structs are `Default + Clone` and can be built with field
overrides.

| Method                    | Request            | Returns                                       |
| ------------------------- | ------------------ | --------------------------------------------- |
| [`Manager::add`]          | [`AddRequest`]     | [`AddOutcome`] (installed + linked + failed)  |
| [`Manager::agent`]        | [`AgentRequest`]   | [`AgentOutcome`] (per-agent results)          |
| [`Manager::agent_status`] | `bool` (global)    | `Vec<`[`AgentStatus`]`>`                      |
| [`Manager::list`]         | [`ListRequest`]    | `Vec<`[`ListedSkill`]`>` (serializable)       |
| [`Manager::remove`]       | [`RemoveRequest`]  | [`RemoveOutcome`] (removed names)             |
| [`Manager::disable`]      | [`DisableRequest`] | [`DisableOutcome`] (disabled names)           |
| [`Manager::enable`]       | [`EnableRequest`]  | [`EnableOutcome`] (enabled names)             |

### Request-struct fields

| Struct             | Fields (besides `global: bool`)                                                                             |
| ------------------ | ----------------------------------------------------------------------------------------------------------- |
| [`AddRequest`]     | `source: String`, `skills: Vec<String>` (`"*"` or specific names, empty = all), `list_only: bool`           |
| [`AgentRequest`]   | `agents: Vec<String>`, `unlink: bool`                                                                       |
| [`ListRequest`]    | — (scope only, via `global: bool`)                                                                          |
| [`RemoveRequest`]  | `skills: Vec<String>`, `all: bool`                                                                          |
| [`DisableRequest`] | `skills: Vec<String>`, `all: bool`                                                                          |
| [`EnableRequest`]  | `skills: Vec<String>`, `all: bool`                                                                          |

All request structs carry a `global: bool` field: `true` operates on the
global `~/.agents/skills` (the CLI default), `false` on the project-level
`./.agents/skills` (the CLI's `--project`). The project root comes from
`Env.cwd` (overridable via `Manager::builder().cwd()`, corresponding to the
CLI's `--project <dir>`). The `agents` field of [`AgentRequest`] restricts the
agents (`"*"` or specific names, empty = auto-detect). Which agents see a
skill is scope-level state, reported by [`Manager::agent_status`] — not a
per-skill property.

### Result-struct fields

[`Manager::list`] returns [`ListedSkill`] values — the exact shape `list --json`
serializes: `name`, `description` (collapsed onto a single line), `path` (the
directory the skill currently lives in: canonical, or `disabled-skills`),
`enabled`, and `installed_at` (Unix seconds, UTC; `None` when the filesystem
records no creation time). `installed_at` approximates when the skill landed on
disk: exact for `add` installs, but a skill adopted from an agent directory
keeps that directory's original creation time.

### Correspondence with the CLI

- **`add` takes a single source**: the CLI's `add <source...>` installs
  several sources at once, while the library's [`AddRequest`] accepts a single
  `source: String`. To install multiple sources, call `manager.add(...)`
  several times; each call returns its own [`AddOutcome`].
- **`AgentRequest` link conventions**: the CLI's `agent` command requires
  exactly one of `--link`/`--unlink`/`--status`; the library splits `--status`
  into the separate [`Manager::agent_status`], so [`AgentRequest`] only
  distinguishes link from unlink: `unlink: false` (default) links,
  `unlink: true` unlinks. Linking *adopts* whatever the agent's skills
  directory already holds, and that is one-way: skill directories are moved
  into the canonical directory, non-skill entries into `.misc/<agent>/` inside
  it, and name clashes are dropped in favour of the existing copy (the
  canonical copy wins; a skill disabled in `disabled-skills` stays disabled
  rather than being re-imported). Unlinking does not move adopted content back.
  [`LinkOutcome::Refused`] is returned only when the agent directory is a
  symlink pointing elsewhere.

### Common operations

```rust
use agents_skills::{AddRequest, DisableRequest, EnableRequest, ListRequest, RemoveRequest};

// Install specific skills / list without installing
let outcome = manager.add(&AddRequest {
    source: "anthropics/skills".into(),
    skills: vec!["pdf".into()],   // omitted = install all
    list_only: false,             // true = only list available skills
    ..Default::default()
})?;

// List skills (the global field picks the scope; --json is the CLI counterpart)
let skills = manager.list(&ListRequest::default())?;
let json = serde_json::to_string_pretty(&skills)?; // the CLI's list --json

// Remove skills
manager.remove(&RemoveRequest { skills: vec!["pdf".into()], ..Default::default() })?;

// Disable / enable (moves the skill directory out of / back into the canonical directory)
manager.disable(&DisableRequest { skills: vec!["pdf".into()], ..Default::default() })?;
manager.enable(&EnableRequest { skills: vec!["pdf".into()], ..Default::default() })?;
```

## Context injection: [`ManagerBuilder`]

```rust
let manager = Manager::builder()
    .home("/tmp/home")
    .config("/tmp/config")
    .cwd("/tmp/project")
    .env_var("CLAUDE_CONFIG_DIR", "/tmp/claude")
    .build();
```

Use it for sandboxes/tests to avoid touching the real environment;
`Manager::new()` equals `Manager::builder().build()`. In a sandbox, adding
`.probe_system_dirs(false)` makes agent probing skip system locations entirely
(e.g. `/Applications/ZCode.app`), keeping results hermetic and reproducible.

## Examples

```bash
cargo run --example manage      # demonstrates add → list → remove on a temp directory (no side effects)
cargo run --example add_skill   # installs into the real environment via the Manager
```

## Behavioral contract

The library stays **pure data**: it never prints, never calls
`process::exit`; results are structured and errors surface through `Result` —
rendering and exit codes are the caller's job. The library has **no
telemetry** — no data ever leaves your machine.

[`Manager`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html
[`Manager::add`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.add
[`Manager::agent`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent
[`Manager::agent_status`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent_status
[`Manager::list`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.list
[`Manager::remove`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.remove
[`Manager::disable`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.disable
[`Manager::enable`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.enable
[`ManagerBuilder`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ManagerBuilder.html
[`AddRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AddRequest.html
[`AddOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AddOutcome.html
[`AgentRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentRequest.html
[`AgentOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentOutcome.html
[`AgentStatus`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentStatus.html
[`LinkOutcome::Refused`]: https://docs.rs/agents-skills/latest/agents_skills/enum.LinkOutcome.html
[`ListRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListRequest.html
[`ListedSkill`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListedSkill.html
[`RemoveRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveRequest.html
[`RemoveOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveOutcome.html
[`DisableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableRequest.html
[`DisableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableOutcome.html
[`EnableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableRequest.html
[`EnableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableOutcome.html
