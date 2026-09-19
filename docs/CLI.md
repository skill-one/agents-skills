# agents-skills CLI Reference

English | [简体中文](CLI.zh-CN.md)

Complete command reference for the `agents-skills` CLI. For a feature overview
see the [README](../README.md); library users see [LIBRARY.md](LIBRARY.md).

## Install

```bash
cargo install agents-skills
```

Global option: `-v, --version` prints the version.

## Command cheat sheet

| Command   | Description                         |
| --------- | ----------------------------------- |
| `add`     | Install a skill pack from a source  |
| `remove`  | Remove installed skills             |
| `list`    | List installed skills               |
| `disable` | Disable an installed skill          |
| `enable`  | Re-enable a disabled skill          |
| `agent`   | Link / unlink / show agent status   |

Commands have no aliases (a minimal interface — full names only).

General notes: skills live in exactly one place — the canonical directory
`~/.agents/skills` — and every command operates on it. Disabled skills are
parked in the sibling `~/.agents/disabled-skills`.

## add

Install a skill pack from a local path, a Git repository, or an HTTPS endpoint.

```
agents-skills add <source...> [options]
```

| Option               | Description                                |
| -------------------- | ------------------------------------------ |
| `-s, --skill <s>...` | Skill names to install (`'*'` = all)       |
| `-l, --list`         | Only list available skills, do not install |

```bash
agents-skills add anthropics/skills               # install into ~/.agents/skills
agents-skills add anthropics/skills@pdf           # install only the specified skill
agents-skills add anthropics/skills -l            # only list available skills
```

`add` only ever adds and never overwrites: a skill whose name is already
installed — enabled *or* disabled — is reported as `skipped` and left as is. To
replace an installed skill, `remove` it first.

After installing, run `agents-skills agent --link` to make the skills visible
to agents (`add` does not link automatically).

## remove

Remove installed skills from the canonical directory.

```
agents-skills remove [skills...] [options]
```

| Option               | Description                                 |
| -------------------- | ------------------------------------------- |
| `-s, --skill <s>...` | Skills to remove (`'*'` = all)              |
| `--all`              | Remove all skills (including disabled ones) |

```bash
agents-skills remove pdf      # remove the specified skill
agents-skills remove --all    # remove all skills
```

## list

List installed skills with each skill's description, install time, and estimated
description token cost. Use `agent --status` for each agent's link status.

```
agents-skills list [options]
```

| Option   | Description                    |
| -------- | ------------------------------ |
| `--json` | JSON output (machine-readable) |

```bash
agents-skills list
agents-skills list --json
```

Each skill is printed on two lines — name and description, then
`path [status] ~N tokens · <local install time>` — followed by the always-on
total for enabled skills:

```
Skills

docx Create and edit Word documents, including tables and headers.
  ~/.agents/skills/docx [enabled] ~16 tokens · 2026-09-19 12:54
Enabled skills keep ~16 tokens of descriptions in context.
```

The token figure sizes a skill's always-on context cost: harnesses keep every
linked skill's name and description in context, while the `SKILL.md` body is
only loaded once the skill triggers. Disabled skills are excluded from the
total (they are parked outside every agent's view). It is a dependency-free
heuristic (~4 ASCII characters, or 1 non-ASCII character, per token), so treat
it as an order-of-magnitude figure rather than an exact count.

`list --json` emits the same fields per skill:

| Field             | Description                                                                    |
| ----------------- | ------------------------------------------------------------------------------ |
| `name`            | Skill name (from `SKILL.md` frontmatter)                                       |
| `description`     | Skill description, collapsed onto a single line                                |
| `estimatedTokens` | Estimated tokens the description costs in an agent's context (see above)       |
| `path`            | Directory the skill currently lives in (canonical, or `disabled-skills`)       |
| `enabled`         | `true` in the canonical directory, `false` parked in `disabled-skills`         |
| `installedAt`     | Skill directory creation time as Unix seconds (UTC), or `null` when unavailable |

`installedAt` approximates when the skill landed on disk: it is exact for `add`
installs, but a skill adopted from an agent directory keeps that directory's
original creation time, and re-installing over a skill refreshes it. It is
`null` on filesystems that record no creation time (some Linux filesystems).

## disable / enable

`disable` moves a skill's directory into `disabled-skills/`, hiding it from all
agents; `enable` moves it back into the canonical directory and restores
visibility (the inverse of `disable`). Files are preserved intact — lossless
and reversible.

```
agents-skills disable [skills...] [options]
agents-skills enable  [skills...] [options]
```

| Option               | Description                               |
| -------------------- | ----------------------------------------- |
| `-s, --skill <s>...` | Target skills (`'*'` = all)               |
| `--all`              | Disable all enabled / enable all disabled |

```bash
agents-skills disable pdf      # disable the specified skill
agents-skills disable --all    # disable all enabled skills
agents-skills enable  pdf      # enable the specified skill
agents-skills enable  --all    # enable all disabled skills
```

## agent

Manage the link between each agent's skills directory and the canonical
directory.

```
agents-skills agent [agents...] (--link | --unlink | --status) [options]
```

| Option     | Description                                                                                    |
| ---------- | ---------------------------------------------------------------------------------------------- |
| `--link`   | Link the agent's skills directory to the canonical directory (pre-existing content is adopted)  |
| `--unlink` | Unlink the agent from the canonical directory (adopted content stays in the canonical dir)      |
| `--status` | Show link status (read-only)                                                                    |

`--link`, `--unlink`, and `--status` are mutually exclusive; exactly one must
be given. `--status` distinguishes two kinds of visibility: agents that read
the canonical directory natively (Codex, Cursor, Warp, ...) are tagged
`(canonical dir)`, while those wired in through a symlink are tagged
`(linked)`. For **unlinked** agents, it categorizes the contents of their own
skills directory: `private skills: ...` are skills (subdirectories and
symlinks pointing to directories), `other files: ...` are other files — that
is, what linking would adopt. Agents default to the auto-detected set; `'*'`
means all.

How linking handles pre-existing content (adoption is one-way):

- Empty directory: replaced by the link directly.

- Otherwise the contents are adopted before the link is created: skill
  directories are moved into the canonical directory, non-skill entries are
  moved into `.misc/<agent>/` inside it (a dot-dir, so they are never mistaken
  for installed skills), and only then is the agent directory replaced by the
  link.

- Name clashes are dropped in favour of the existing copy: the canonical copy
  wins, and a name disabled in `disabled-skills` stays disabled instead of
  being re-imported. Legacy per-skill symlinks pointing into the canonical
  directory are dropped as well — their content already lives there.

- Adopted content is *not* restored by `--unlink`; it is managed by
  `remove`/`disable` from then on.

- Refused in only one case: the directory itself is a symlink pointing
  elsewhere.

```bash
agents-skills agent --link                       # link all installed agents
agents-skills agent --link claude-code           # link (pre-existing content is adopted)
agents-skills agent --status                     # show link status
agents-skills agent --unlink claude-code         # unlink the specified agent
```

## Related concepts

- Source formats and install locations: see
  [README · How it works](../README.md#how-it-works).

- Library API (the `Manager` method and request/result types behind each
  command): see [LIBRARY.md](LIBRARY.md).

