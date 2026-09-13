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

General notes: skills live in the canonical directory (global
`~/.agents/skills` or project `.agents/skills`). Commands operate on the
**global** scope by default; `-p/--project <dir>` switches to the **project**
scope — the value is required and the directory must already exist, and
commands then operate on the `.agents/skills` inside it (use `--project .` for
the current directory).

## add

Install a skill pack from a local path, a Git repository, or an HTTPS endpoint.

```
agents-skills add <source...> [options]
```

| Option                | Description                                                |
| --------------------- | ---------------------------------------------------------- |
| `-p, --project <dir>` | Install into the given project directory (default: global) |
| `-s, --skill <s>...`  | Skill names to install (`'*'` = all)                       |
| `-l, --list`          | Only list available skills, do not install                 |

```bash
agents-skills add anthropics/skills               # install into the global ~/.agents/skills
agents-skills add anthropics/skills --project .   # install into the current project's .agents/skills
agents-skills add anthropics/skills@pdf           # install only the specified skill
agents-skills add anthropics/skills -l            # only list available skills
```

After installing, run `agents-skills agent --link` to make the skills visible
to agents (`add` does not link automatically).

## remove

Remove installed skills from the canonical directory.

```
agents-skills remove [skills...] [options]
```

| Option                | Description                                               |
| --------------------- | --------------------------------------------------------- |
| `-p, --project <dir>` | Remove from the given project directory (default: global) |
| `-s, --skill <s>...`  | Skills to remove (`'*'` = all)                            |
| `--all`               | Remove all skills (including disabled ones)               |

```bash
agents-skills remove pdf      # remove the specified skill
agents-skills remove --all    # remove all skills
```

## list

List installed skills (including enabled/disabled status). Use
`agent --status` for each agent's link status.

```
agents-skills list [options]
```

| Option                | Description                                                  |
| --------------------- | ------------------------------------------------------------ |
| `-p, --project <dir>` | List skills in the given project directory (default: global) |
| `--json`              | JSON output (machine-readable, includes the `enabled` field) |

```bash
agents-skills list
agents-skills list --json
agents-skills list --project .
```

## disable / enable

`disable` moves a skill's directory into `disabled-skills/`, hiding it from all
agents; `enable` moves it back into the canonical directory and restores
visibility (the inverse of `disable`). Files are preserved intact — lossless
and reversible.

```
agents-skills disable [skills...] [options]
agents-skills enable  [skills...] [options]
```

| Option                | Description                                                  |
| --------------------- | ------------------------------------------------------------ |
| `-p, --project <dir>` | Project scope: the given project directory (default: global) |
| `-s, --skill <s>...`  | Target skills (`'*'` = all)                                  |
| `--all`               | Disable all enabled / enable all disabled                    |

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

| Option                | Description                                                                                                              |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `-p, --project <dir>` | Operate on the skills directory under the given project directory (default: global)                                      |
| `--link`              | Link the agent's skills directory to the canonical directory (pre-existing content is backed up automatically)           |
| `--unlink`            | Unlink the agent from the canonical directory and restore the backed-up content                                          |
| `--status`            | Show link status (read-only)                                                                                             |
| `--migrate`           | Migrate pre-existing skills into the canonical directory, including those parked in the backup slot (only with `--link`) |

`--link`, `--unlink`, and `--status` are mutually exclusive; exactly one must
be given. `--status` distinguishes two kinds of visibility: agents that read
the canonical directory natively (Codex, Cursor, Warp, ...) are tagged
`(canonical dir)`, while those wired in through a symlink are tagged
`(linked)`. For **unlinked** agents, it categorizes the contents of their own
skills directory: `private skills: ...` are skills (subdirectories and
symlinks pointing to directories), `other files: ...` are other files; if a
backup is waiting to be restored, it is shown as
`backup parked at <path> (<items>) — unlink restores`.
Agents default to the auto-detected set; `'*'` means all.

How linking handles pre-existing content (data is never destroyed):

- Empty directory: replaced by the link directly.

- Otherwise: the whole skills directory is moved as-is into the backup slot
  `.agents/backup-skills/<agent>/skills/` (project-level under
  `./.agents/backup-skills/`, global under `~/.agents/backup-skills/`; the slot
  contains a `manifest.json`), then the link is created; `--unlink` restores
  everything with a single atomic rename.

- With `--migrate`: after the backup, the skill directories inside are moved
  into the canonical directory; on name conflicts the canonical copy wins, and
  skills disabled in `disabled-skills` stay disabled (the agent-side copy stays
  in the backup).

- Refused in only two cases: the directory itself is a symlink pointing
  elsewhere; or the backup from the previous link has not been restored yet.

```bash
agents-skills agent --link                       # link all installed agents
agents-skills agent --link claude-code           # link (pre-existing content is backed up automatically)
agents-skills agent --link claude-code --migrate # link and migrate pre-existing skills into the canonical directory
agents-skills agent --status                     # show link status
agents-skills agent --unlink claude-code         # unlink the specified agent and restore its backup
```

## Related concepts

- Source formats and install locations: see
  [README · How it works](../README.md#how-it-works).

- Library API (the `Manager` method and request/result types behind each
  command): see [LIBRARY.md](LIBRARY.md).

