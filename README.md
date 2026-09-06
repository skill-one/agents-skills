# agents-skills

[![crates.io](https://img.shields.io/crates/v/agents-skills.svg)](https://crates.io/crates/agents-skills)
[![docs.rs](https://img.shields.io/docsrs/agents-skills.svg)](https://docs.rs/agents-skills)
[![CI](https://github.com/skill-one/agents-skills/actions/workflows/ci.yml/badge.svg)](https://github.com/skill-one/agents-skills/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

English | [简体中文](README.zh-CN.md)

A minimal installer and manager for AI agent skills: all skills live in one
**canonical directory**, and a single `agent --link` makes them visible and
usable to [Claude Code](https://claude.com/code), Codex, Cursor, and 70+ other
coding agents — install once, works everywhere.

```bash
cargo install agents-skills
```

Also ships as an embeddable Rust library — see [docs/LIBRARY.md](docs/LIBRARY.md).

## Quick start

```bash
agents-skills agent --link            # link all installed agents
agents-skills add anthropics/skills   # install a skill pack (visible to every agent immediately once it lands in the canonical directory)
agents-skills list                    # list installed skills
agents-skills agent --status          # show the link status of each agent
```

The core idea is linking: skills are stored exactly once in the canonical
directory (project-level `.agents/skills/` or global `~/.agents/skills/`), and
`agent --link` creates, for every installed agent, a symlink in its skills
directory pointing at the canonical directory; skills installed afterwards via
`add` become visible to all agents immediately — no syncing needed.

## How it works

`add`/`remove`/`update`/`disable`/`enable` only operate on the canonical
directory; agents share it automatically through the symlinks. Common commands:

```bash
agents-skills agent --link claude-code            # link (pre-existing content is backed up automatically)
agents-skills agent --link claude-code --migrate  # link and migrate pre-existing skills into the canonical directory
agents-skills agent --unlink claude-code          # unlink (and restore the backed-up content)
agents-skills add anthropics/skills@pdf            # install only the specified skill
agents-skills list --json                          # machine-readable output
agents-skills remove pdf                           # remove a skill
agents-skills update                               # update to the latest versions per the lockfile
agents-skills disable pdf                          # disable (moved out of the canonical directory, files kept)
agents-skills enable pdf                           # re-enable (inverse of disable)
```

- Commands operate on the global scope `~/.agents/skills` by default;
  `--project <dir>` switches to the project scope (`.agents/skills` under the
  given directory; use `--project .` for the current directory).
- When an agent's skills directory already has content, `agent --link` does not
  refuse: the whole directory is moved as-is into the backup slot
  `.agents/backup-skills/<agent>/skills/`, and `agent --unlink` restores it in
  full; with `--migrate`, the skills inside are moved into the canonical
  directory (on name conflicts the canonical copy wins, and the agent-side copy
  stays in the backup).
- For unlinked agents, `agent --status` categorizes the contents of their own
  skills directory (`private skills` / `other files`) plus any backup waiting
  to be restored (`backup parked at`); content of linked/canonical agents is
  shown by `list`.
- `update` skips disabled skills; `list` always shows every skill, tagged
  `enabled`/`disabled`.

### Source formats

The `<source>` argument of `add` accepts:

| Format              | Example                                                          |
| ------------------- | ---------------------------------------------------------------- |
| Local path          | `./my-skill`, `/abs/path/skill`                                  |
| GitHub shorthand    | `owner/repo`                                                     |
| GitHub / GitLab URL | `https://github.com/owner/repo`, `https://gitlab.com/group/repo` |
| SSH / git URL       | `git@github.com:owner/repo.git`                                  |
| HTTPS download      | `https://example.com/skills.zip`, `.../skills.tar.gz`, raw `SKILL.md` |

Common ways to narrow a GitHub source (GitLab works the same, with
`/-/tree/...` instead of `/tree/...`):

| You want                            | Write                                                 |
| ----------------------------------- | ----------------------------------------------------- |
| Everything in the repo              | `owner/repo`                                          |
| Only one skill                      | `owner/repo@pdf` (same as `--skill pdf`)              |
| Only a subdirectory                 | `owner/repo/skills/pdf`                               |
| A branch, tag, or commit            | `https://github.com/owner/repo/tree/v1.2`             |
| A subdirectory at a specific ref    | `https://github.com/owner/repo/tree/v1.2/skills/pdf`  |
| One skill from a subdirectory       | `agents-skills add owner/repo/skills/pdf --skill pdf` |

Rules for Git sources:

- A ref (branch / tag / commit SHA) can only be expressed in the URL:
  `/tree/<ref>[/<path>]` for GitHub, `/-/tree/<ref>[/<path>]` for GitLab
  (including self-hosted instances). The shorthand, plain repo URLs, and
  SSH / git URLs have no ref syntax and always resolve to the default branch.
- The shorthand cannot combine a subdirectory with `@skill` — use `--skill`
  instead. GitHub / GitLab URLs without `/tree/` (e.g.
  `https://github.com/owner/repo/skills/pdf`) and `/blob/` file URLs are
  rejected with an error instead of silently installing the wrong scope.

HTTPS sources are downloaded and extracted directly (zip / tar / tar.gz
archive, or a single file such as a raw `SKILL.md`).

Within a repository, skills are discovered in priority-ordered container
directories (`skills/`, `.curated/`, `.experimental/`, `.system/`); shallower
directories shadow deeper ones.

### Install locations

- **Canonical directory (the single real copy)** — project-level
  `./.agents/skills/<name>`, global `~/.agents/skills/<name>`.
- **Agent integration** — agents that do not natively read the canonical
  directory get a directory-level symlink: `.claude/skills` →
  `../.agents/skills` (project) or `~/.claude/skills` → `~/.agents/skills`
  (global).

## Command cheat sheet

| Command   | Description                            |
| --------- | -------------------------------------- |
| `add`     | Install a skill pack from a source     |
| `remove`  | Remove installed skills                |
| `list`    | List installed skills                  |
| `update`  | Update skills to the latest version    |
| `disable` | Disable an installed skill             |
| `enable`  | Re-enable a disabled skill             |
| `agent`   | Link / unlink / show agent link status |

Commands have no aliases (a minimal interface — full names only).

> For the full command-line reference see [docs/CLI.md](docs/CLI.md); library
> users see [docs/LIBRARY.md](docs/LIBRARY.md); project developers see
> [docs/DEVELOPER.md](docs/DEVELOPER.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
