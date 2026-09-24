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
│   ├── source.rs       Source-string parsing (local dir or `owner/repo@<skill>`)
│   ├── agents.rs       Declarative interpreter over the agent table (resolution + detection)
│   ├── agents.jsonl    The agent table: one JSON object per agent line
│   ├── discover.rs     Skill discovery: name = directory name, best-effort description read
│   ├── github.rs       GitHub tarball fetching (codeload single request + local directory matching)
│   ├── install.rs      Install skills into the canonical directory + installed-skills listing
│   ├── link/           Directory-level agent linking (link/unlink)
│   │   ├── mod.rs      Link orchestration + adoption of pre-existing content
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
  "name": "claude-code", // required, unique identifier (used on the CLI)
  "display": "Claude Code", // required, human-readable name
  "global": {
    "env_home": {
      "var": "CLAUDE_CONFIG_DIR",
      "default": ".claude",
      "path": "skills",
    },
  },
  "detect": [
    { "env_home": { "var": "CLAUDE_CONFIG_DIR", "default": ".claude" } },
  ],
}
```

`global` is a single path spec, and `detect` is a list of path specs — an agent
is detected as installed when any one of them resolves to an existing path.
Exactly one of these keys per spec:

| Key                                                             | Resolves to                                          |
| --------------------------------------------------------------- | ---------------------------------------------------- |
| `{"home": "..."}`                                               | `home/<path>`                                        |
| `{"config": "..."}`                                             | `config/<path>`                                      |
| `{"cwd": "..."}`                                                | `cwd/<path>`                                         |
| `{"env_home": {"var": "...", "default": "...", "path": "..."}}` | `$VAR \|\| home/<default>`, then `<path>` joined     |
| `{"env_var": {"var": "...", "path": "..."}}`                    | `$VAR/<path>`; unmatched when the var is unset       |
| `{"system": "/abs/path"}`                                       | absolute path; only probed when system probing is on |

Whether an agent needs a symlink is decided by `is_native` in `agents.rs`: the
resolved `global` spec is compared against `~/.agents/skills` — only agents
whose dir equals it (e.g. cline, warp) are native and need no link; agents with
a vendor-specific dir (e.g. Antigravity's `~/.gemini/config/skills`) get a
real directory symlink. The `universal` pseudo-agent carries `"detect": []` so
it is never detected as installed.

## Development & testing

```bash
cargo build            # build
cargo test             # run all tests
cargo clippy           # lint
cargo fmt              # format
```

- **Unit tests** live inline in `src/` modules (`#[cfg(test)]`); shared
  fixtures are in `src/core/test_utils.rs`.
- **Integration tests** in `tests/` drive the real CLI via `assert_cmd`;
  `lib_api.rs` covers the library API.
- Examples: `cargo run --example manage` (lifecycle on a temp directory) and
  `cargo run --example add_skill` (real environment).

## Releasing

Releases to crates.io always go through GitHub Actions (see
`.github/workflows/`) — never run `cargo publish` manually. Before releasing:
bump `version` in `Cargo.toml` per semantic versioning, record the change in
`CHANGELOG.md` (and `CHANGELOG.zh-CN.md`), and keep the `*.zh-CN.md`
translations in sync.
