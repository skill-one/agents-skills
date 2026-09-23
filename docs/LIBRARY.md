# agents-skills Library Guide

English | [简体中文](LIBRARY.zh-CN.md)

Embed the skill-manager into your own Rust tooling. For CLI usage see the
[README](../README.md); the full command reference is [CLI.md](CLI.md).

## Adding the dependency

```toml
[dependencies]
agents-skills = "0.21"
```

## Quick start

```rust
use agents_skills::{AddRequest, Manager};

let manager = Manager::new();
let outcome = manager.add(&AddRequest::new("anthropics/skills@pdf"))?;
println!("{} (skipped={})", outcome.skill.name, outcome.skipped);
let skills = manager.list()?;
```

## API surface: [`Manager`]

Every method takes a `Default + Clone` request struct and returns a structured
result.

| Method                    | Request            | Returns                                   |
| ------------------------- | ------------------ | ----------------------------------------- |
| [`Manager::add`]          | [`AddRequest`]     | [`AddOutcome`] (one skill + skipped flag) |
| [`Manager::agent`]        | [`AgentRequest`]   | [`AgentOutcome`] (per-agent results)      |
| [`Manager::agent_status`] | —                  | `Vec<`[`AgentStatus`]`>`                  |
| [`Manager::list`]         | —                  | `Vec<`[`ListedSkill`]`>` (serializable)   |
| [`Manager::remove`]       | [`RemoveRequest`]  | [`RemoveOutcome`] (removed names)         |
| [`Manager::disable`]      | [`DisableRequest`] | [`DisableOutcome`] (disabled names)       |
| [`Manager::enable`]       | [`EnableRequest`]  | [`EnableOutcome`] (enabled names)         |

Request fields:

| Struct             | Fields                                                                                                           |
| ------------------ | ---------------------------------------------------------------------------------------------------------------- |
| [`AddRequest`]     | `source: String` (a local skill directory or `owner/repo@<skill>`), `reference: Option<String>` (branch/tag/SHA) |
| [`AgentRequest`]   | `agents: Vec<String>` (`"*"` or names, empty = auto-detect), `unlink: bool`                                      |
| [`RemoveRequest`]  | `skills: Vec<String>`, `all: bool`                                                                               |
| [`DisableRequest`] | `skills: Vec<String>`, `all: bool`                                                                               |
| [`EnableRequest`]  | `skills: Vec<String>`, `all: bool`                                                                               |

Notes:

- [`AddOutcome`] is one skill: `source`, `skill`, `canonical_path`, `skipped`
  (`true` when the name already exists — `add` never overwrites). Failures are
  `Err`. [`ListedSkill`] has the same fields as `list --json` (see
  [CLI.md](CLI.md#list)); resolve a skill's directory with
  [`Manager::skill_dir`].
- The CLI's `--link`/`--unlink`/`--status` split into [`Manager::agent`] and
  [`Manager::agent_status`] — [`AgentRequest`] only carries `unlink: bool`.
  Adoption semantics (one-way, canonical copy wins) are the same as the CLI's;
  [`LinkOutcome::Refused`] is returned only when the agent directory itself is
  a symlink pointing elsewhere.

### Common operations

```rust
use agents_skills::{AddRequest, DisableRequest, EnableRequest, RemoveRequest};

// Pin a branch/tag/SHA with `reference` (None = default branch).
manager.add(&AddRequest {
    source: "anthropics/skills@pdf".into(),
    reference: Some("v1.2".into()),
    ..Default::default()
})?;

let skills = manager.list()?;
let json = serde_json::to_string_pretty(&skills)?; // same shape as list --json

manager.remove(&RemoveRequest  { skills: vec!["pdf".into()], ..Default::default() })?;
manager.disable(&DisableRequest{ skills: vec!["pdf".into()], ..Default::default() })?;
manager.enable(&EnableRequest  { skills: vec!["pdf".into()], ..Default::default() })?;
```

## Context injection: [`ManagerBuilder`]

```rust
let manager = Manager::builder()
    .home("/tmp/home")
    .config("/tmp/config")
    .cwd("/tmp/project")
    .env_var("CLAUDE_CONFIG_DIR", "/tmp/claude")
    .probe_system_dirs(false) // skip system locations for hermetic tests
    .build();
```

`Manager::new()` equals `Manager::builder().build()`. Runnable examples:

```bash
cargo run --example manage      # add → list → remove on a temp directory (no side effects)
cargo run --example add_skill   # installs into the real environment
```

## Behavioral contract

The library is pure data: it never prints and never calls `process::exit`;
rendering and exit codes are the caller's job. It has no telemetry — no data
leaves the machine.

[`Manager`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html
[`Manager::add`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.add
[`Manager::agent`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent
[`Manager::agent_status`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent_status
[`Manager::list`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.list
[`Manager::skill_dir`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.skill_dir
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
[`ListedSkill`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListedSkill.html
[`RemoveRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveRequest.html
[`RemoveOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveOutcome.html
[`DisableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableRequest.html
[`DisableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableOutcome.html
[`EnableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableRequest.html
[`EnableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableOutcome.html
