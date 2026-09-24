# agents-skills CLI Reference

English | [简体中文](CLI.zh-CN.md)

Complete command reference for the `agents-skills` CLI. For a feature overview
see the [README](../README.md); library users see [LIBRARY.md](LIBRARY.md).

Global option: `-v, --version`. Every command operates on the canonical
directory `~/.agents/skills`; disabled skills are parked in
`~/.agents/disabled-skills`. Commands have no aliases — full names only.

## add

Install exactly one skill from a local directory or from GitHub.

```
agents-skills add <source> [--ref <ref>]
```

`<source>` is either a local skill directory (it must directly contain a
`SKILL.md`) or `owner/repo@<skill>`, naming one skill on GitHub. A skill's
name is always its directory name: the repository tree is searched for a
directory containing `SKILL.md` whose basename matches `<skill>`
case-insensitively (shallowest match wins). A `SKILL.md` at the repository
root is selected with the repository name and installs the whole repository.
The frontmatter `name` field is ignored everywhere.

| Option        | Description                                            |
| ------------- | ------------------------------------------------------ |
| `--ref <ref>` | Pin a branch, tag, or full commit SHA (GitHub sources only) |

```bash
agents-skills add ./my-skill                        # install a local skill
agents-skills add anthropics/skills@pdf             # install one skill from GitHub
agents-skills add anthropics/skills@pdf --ref v1.2  # pin a branch, tag, or full commit SHA
```

Remote installs are a single request: the whole repository tarball is
downloaded from `codeload.github.com`, unpacked into a temp dir, and the
matched skill directory is selected locally. The GitHub REST API is never
used, so there is no API rate limit; public repositories only, and Git LFS
files install as their pointer stubs. Without `--ref` the default branch is
used. Any other source form
(bare `owner/repo`, full URLs, subpaths, GitLab/SSH, HTTPS archives) is rejected
with a message naming the supported syntax.

`add` never overwrites: a name already installed — enabled or disabled — is
reported `skipped`; `remove` it first to replace it. Run `agent --link` after
installing to make the skill visible to agents.

## remove

Remove skills from the canonical directory (and any parked copy under
`disabled-skills`).

```
agents-skills remove [skills...] [--all]
```

| Option               | Description                                 |
| -------------------- | ------------------------------------------- |
| `-s, --skill <s>...` | Skill names to remove                       |
| `--all`              | Remove all skills (including disabled ones) |

```bash
agents-skills remove pdf      # remove the specified skill
agents-skills remove --all    # remove all skills
```

## list

List installed skills with description, path, status and install time.

```
agents-skills list [--json]
```

Each skill is printed on two lines — name and description, then
`path [status] · <local install time>`:

```
Skills

docx Create and edit Word documents, including tables and headers.
  ~/.agents/skills/docx [enabled] · 2026-09-19 12:54
```

`--json` emits one object per skill:

| Field         | Description                                                                                     |
| ------------- | ----------------------------------------------------------------------------------------------- |
| `name`        | Skill directory name — the identity every command uses; the frontmatter `name` field is ignored |
| `description` | Skill description, collapsed onto a single line                                                 |
| `enabled`     | `true` in the canonical directory, `false` parked in `disabled-skills`                          |
| `installedAt` | Skill directory creation time as Unix seconds (UTC), or `null` when unavailable                 |

Use `agent --status` for each agent's link status.

## disable / enable

`disable` moves a skill's directory into `disabled-skills/`, hiding it from all
agents; `enable` moves it back. Files are preserved — lossless and reversible.

```
agents-skills disable [skills...] [--all]
agents-skills enable  [skills...] [--all]
```

| Option               | Description                               |
| -------------------- | ----------------------------------------- |
| `-s, --skill <s>...` | Skill names to disable / enable           |
| `--all`              | Disable all enabled / enable all disabled |

Idempotent: repeating a command reports the skill as already in that state;
unknown names are reported as missing, not errors.

## agent

Manage the link between each agent's skills directory and the canonical
directory.

```
agents-skills agent [agents...] (--link | --unlink | --status)
```

| Option     | Description                                                                                    |
| ---------- | ---------------------------------------------------------------------------------------------- |
| `--link`   | Link the agent's skills directory to the canonical directory (pre-existing content is adopted) |
| `--unlink` | Unlink from the canonical directory (adopted content stays there)                              |
| `--status` | Show link status (read-only)                                                                   |

The three modes are mutually exclusive. Agents default to the auto-detected
set; `'*'` means all. `--status` tags agents that read the canonical directory
natively as `(canonical dir)` and symlinked ones as `(linked)`; for unlinked
agents it categorizes their own directory contents — `private skills` vs
`other files` — i.e. what linking would adopt.

How `--link` handles pre-existing content (adoption is one-way):

- An empty directory is replaced by the link directly.
- Otherwise contents are adopted first: skill directories move into the
  canonical directory, other entries into its `.misc/<agent>/` (a dot-dir so
  they are never mistaken for skills), then the agent directory becomes the
  link.
- Name clashes keep the existing copy (the canonical one wins; a disabled name
  stays disabled). Legacy per-skill symlinks into the canonical directory are
  dropped — their content already lives there.
- `--unlink` does not restore adopted content; manage it with
  `remove`/`disable` afterwards.
- Refused in only one case: the directory itself is a symlink pointing
  elsewhere.

```bash
agents-skills agent --link                       # link all installed agents
agents-skills agent --link claude-code           # link (pre-existing content is adopted)
agents-skills agent --status                     # show link status
agents-skills agent --unlink claude-code         # unlink the specified agent
```
