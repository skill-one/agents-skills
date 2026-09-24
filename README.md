# agents-skills

[![crates.io](https://img.shields.io/crates/v/agents-skills.svg)](https://crates.io/crates/agents-skills)
[![docs.rs](https://img.shields.io/docsrs/agents-skills.svg)](https://docs.rs/agents-skills)
[![CI](https://github.com/skill-one/agents-skills/actions/workflows/ci.yml/badge.svg)](https://github.com/skill-one/agents-skills/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

English | [简体中文](README.zh-CN.md)

A minimal installer and manager for AI agent skills: all skills live in one
**canonical directory**, and a single `agent --link` makes them visible to
[Claude Code](https://claude.com/code), Codex, Cursor, and 70+ other coding
agents — install once, works everywhere.

```bash
cargo install agents-skills
```

Also ships as an embeddable Rust library — see [docs/LIBRARY.md](docs/LIBRARY.md).

## Quick start

```bash
agents-skills agent --link                  # link all installed agents
agents-skills add anthropics/skills@pdf     # install one skill
agents-skills list                          # list installed skills
```

## How it works

Every skill is stored exactly once in the canonical directory
`~/.agents/skills/<name>` (disabled skills in
`~/.agents/disabled-skills/<name>`). `agent --link` points each installed
agent's skills directory at it with a symlink, so skills installed afterwards
are visible to all agents immediately — no syncing.

```bash
agents-skills add owner/repo@pdf            # install from GitHub
agents-skills add ./my-skill                # install a local skill directory
agents-skills add owner/repo@pdf --ref v1.2 # pin a branch, tag, or commit SHA
agents-skills list --json                   # machine-readable output
agents-skills remove pdf                    # remove a skill
agents-skills disable pdf                   # disable (files kept) / enable: inverse
agents-skills agent --link claude-code      # link one agent
agents-skills agent --unlink claude-code    # unlink (adopted content stays)
agents-skills agent --status                # show link status and private content
```

Key behaviors:

- **Sources** — `add` installs exactly one named skill: either a local
  directory that directly contains `SKILL.md`, or `owner/repo@<skill>` from
  GitHub (matched on the skill directory name, case-insensitively; a
  root-level `SKILL.md` is selected with the repository name). Without `--ref`
  the default branch is used. A remote install is a **single request** that
  downloads the repository tarball from `codeload.github.com` — the GitHub
  REST API is never used, so there is no API rate limit. Public repositories
  only; Git LFS files install as their pointer stubs.
- **Never overwrite** — a skill already installed (enabled or disabled) is
  reported `skipped`; `remove` it first to replace it.
- **Linking adopts existing content one-way** — skill directories move into
  the canonical directory, other files into its `.misc/<agent>/`, and name
  clashes keep the existing copy. `--unlink` disconnects but does not move
  content back.

## Documentation

- [docs/CLI.md](docs/CLI.md) — full command reference
- [docs/LIBRARY.md](docs/LIBRARY.md) — embeddable Rust library
- [docs/DEVELOPER.md](docs/DEVELOPER.md) — architecture and contributing
- [CHANGELOG.md](CHANGELOG.md) — release notes ([中文版](CHANGELOG.zh-CN.md))

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
