# agents-skills Developer Guide

English | [简体中文](DEVELOPER.zh-CN.md)

For **project developers**: project layout, development workflow, testing, and
releasing. For a feature overview and CLI usage see the
[README](../README.md), for the command reference see [CLI.md](CLI.md), for
the library API see [LIBRARY.md](LIBRARY.md).

## Architecture layers

The project is deliberately layered, with a strict separation between the
library and the CLI:

- **Library** (`src/lib.rs` + `src/manager/` + `src/core/`) — pure data:
  never prints, never calls `process::exit`, and surfaces errors through
  `Result`.
- **CLI** (`src/main.rs` + `src/cli.rs` + `src/commands/`) — a thin rendering
  layer on top of the library: it only splits clap arguments, hands request
  structs to the `Manager`, then renders results as human/machine-readable
  output and decides the exit code.

Every CLI command maps to one `Manager` method, and CLI flags map to
request-struct fields. When adding capabilities, implement them first in the
`core`/`Manager` layer, then render them in the CLI layer; never let the CLI
layer touch domain logic directly.

## Project layout

```
src/
├── lib.rs              Library root: Manager facade + request/result types + private core module
├── manager/            High-level Manager facade (add/list/remove/disable/enable/link)
│   ├── mod.rs          Manager methods (one per CLI command)
│   ├── types.rs        Request/outcome structs shared with the CLI layer
│   ├── select.rs       Selection helpers (skill matching + agent resolution)
│   └── tests.rs        Unit tests for the selection helpers
├── error.rs            Unified error type and Result alias
├── core/               Domain logic (pure functions, injectable dependencies)
│   ├── mod.rs          Module organization and re-exports
│   ├── source.rs       Source-string parsing
│   ├── agents.rs       Declarative interpreter over the agent table (resolution + detection)
│   ├── agents.jsonl    The agent table: one JSON object per agent line
│   ├── discover.rs     SKILL.md discovery + frontmatter parsing
│   ├── fetch.rs        git clone / HTTP download / archive unpacking
│   ├── github.rs       GitHub API single-skill fast path
│   ├── install.rs      Install skills into the canonical directory + installed-skills listing
│   ├── link/           Directory-level agent linking (link/unlink/migrate)
│   │   ├── mod.rs      Link orchestration + public entry points
│   │   ├── backup.rs   Backup slots: park/unpark + migration of pre-existing dirs
│   │   ├── outcome.rs  LinkOutcome result enum
│   │   ├── path.rs     Path classification helpers
│   │   └── tests.rs    Unit tests for the linking machinery
│   └── test_utils.rs   Shared unit-test fixtures
├── main.rs             bin entry point (thin CLI on top of the library)
├── cli.rs              clap command tree (commands, flags — no aliases)
└── commands/           CLI rendering layer (argument splitting + output only)
    ├── mod.rs
    ├── add.rs
    ├── remove.rs
    ├── list.rs
    ├── disable.rs
    ├── enable.rs
    └── agent.rs

examples/
├── add_skill.rs        Install a skill via the Manager facade (real usage)
└── manage.rs           Demonstrates the add → list → remove lifecycle on a temp directory

tests/
├── common/mod.rs       Shared integration-test fixtures
├── lib_api.rs          Library API integration tests
├── cli_add.rs
├── cli_remove.rs
├── cli_list.rs
├── cli_agent.rs
├── cli_enable_disable.rs
└── cli_version.rs
```

## Adding an agent

The agent table lives in `src/core/agents.jsonl` — one JSON object per agent,
embedded into the binary at compile time (`include_str!`). Adding, changing, or
removing an agent is a one-line edit in that file; no Rust changes are required.
Blank lines and `#` comments are allowed, and the file order defines the
listing order.

```jsonc
{
  "name": "claude-code",      // required, unique identifier (used on the CLI)
  "display": "Claude Code",   // required, human-readable name
  "skills_dir": ".claude/skills", // required, project-level skills dir (relative to cwd)
  "global": { "env_home": { "var": "CLAUDE_CONFIG_DIR", "default": ".claude", "path": "skills" } },
  "detect": [ { "env_home": { "var": "CLAUDE_CONFIG_DIR", "default": ".claude" } } ],
  "hidden": false             // optional, hide from the universal agents list (default false)
}
```

`global` is a single path spec, and `detect` is a list of path specs — an agent
is detected as installed when any one of them resolves to an existing path.
Exactly one of these keys per spec:

| Key | Resolves to |
| --- | ----------- |
| `{"home": "..."}` | `home/<path>` |
| `{"config": "..."}` | `config/<path>` |
| `{"cwd": "..."}` | `cwd/<path>` |
| `{"env_home": {"var": "...", "default": "...", "path": "..."}}` | `$VAR \|\| home/<default>`, then `<path>` joined |
| `{"env_var": {"var": "...", "path": "..."}}` | `$VAR/<path>`; unmatched when the var is unset |
| `{"system": "/abs/path"}` | absolute path; only probed when system probing is on |

Whether an agent needs a symlink is decided **per scope** (`is_native` in
`agents.rs`): at project scope, agents whose `skills_dir` is `.agents/skills`
share the canonical dir and need no link; at global scope, the resolved
`global` spec is compared against `~/.agents/skills` — only agents whose global
dir equals it (e.g. cline, warp) are native globally. Agents with a
vendor-specific global dir (e.g. Antigravity's `~/.gemini/config/skills`) are
project-universal but get a real directory symlink at global scope. The
`universal` pseudo-agent carries `"detect": []` so it is never detected as
installed.

## Development

```bash
cargo build            # build
cargo test             # run all tests
cargo clippy           # lint
cargo fmt              # format
```

## Testing

Tests follow the test pyramid:

- **Unit tests** — inline in the `src/` modules via `#[cfg(test)]`, fast and
  isolated; domain-layer fixtures live in `src/core/test_utils.rs`.
- **Integration tests** — black-box tests in `tests/` that drive the real CLI
  via `assert_cmd`; `lib_api.rs` covers the library API.

Example programs complement the tests:

```bash
cargo run --example manage      # demonstrates add → list → remove on a temp directory (no side effects)
cargo run --example add_skill   # installs into your real environment via the Manager
```

## Design trade-offs

- **Minimal and stable** — deliberately kept small and stable, with
  cross-platform care (macOS, Linux, Windows).
- **Pure data** — the library never prints, never calls `process::exit`;
  results are structured and errors surface through `Result`.
- **No telemetry** — no data ever leaves the user's machine.

## Releasing

Releases to crates.io always go through GitHub Actions (see
`.github/workflows/`) — never run `cargo publish` manually. Before releasing,
make sure the `version` in `Cargo.toml` has been bumped according to semantic
versioning, update the version numbers and API changes described in the
[README](../README.md) / [CLI.md](CLI.md) / [LIBRARY.md](LIBRARY.md), and keep
the Chinese translations (the `*.zh-CN.md` files alongside each document) in
sync.
