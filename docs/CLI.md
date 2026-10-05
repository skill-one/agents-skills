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
agents-skills add <source> [--ref <ref>] [--force]
```

`<source>` is either a local skill directory or the GitHub id
`owner/repo/slug`, naming one skill on GitHub. In both cases a **skill** is a
directory whose `SKILL.md` frontmatter declares a non-empty `name` — anything
else is rejected as a local source, and never discovered remotely.

The id's last segment is the skill's **slug**: its frontmatter `name`
slugified (lowercase, every space replaced by `-`, `/` dropped, everything
else kept verbatim — e.g. `agent development` → `agent-development`). Every
`SKILL.md` in the repository tree is inspected and the first manifest whose
slugified `name` equals the requested slug wins (shallowest, then path
order). A `SKILL.md` at the repository root is an ordinary candidate: when it
matches, the whole repository is the skill. The installed directory keeps
the matched directory's own name in the source repository, verbatim — while
the skill's identity, what `list` reports and `remove`/`enable`/`disable`
select by, is the slug; the declared frontmatter `name` is shown alongside
as a display rendition (`displayName` in `list --json`).

| Option          | Description                                                 |
| --------------- | ----------------------------------------------------------- |
| `--ref <ref>`   | Pin a branch, tag, or full commit SHA (GitHub sources only) |
| `-f, --force`   | Overwrite an already installed skill instead of skipping it |

```bash
agents-skills add ./my-skill                        # install a local skill
agents-skills add anthropics/skills/pdf             # install one skill from GitHub
agents-skills add anthropics/skills/pdf --ref v1.2  # pin a branch, tag, or full commit SHA
agents-skills add anthropics/skills/pdf --force     # overwrite an already installed skill
```

Remote installs are a single request: the whole repository tarball is
streamed from `codeload.github.com` into a temp dir without high memory overhead,
and the matched skill directory is selected locally. The GitHub REST API is never
used, so there is no API rate limit; public repositories only, and Git LFS
files install as their pointer stubs. Without `--ref` the default branch is
used. Any other source form
(bare `owner/repo`, full URLs, subpaths, GitLab/SSH, HTTPS archives) is rejected
with a message naming the supported syntax.

By default `add` never overwrites: a name already installed — enabled or disabled — is
reported `skipped`; pass `--force` (or `-f`) to overwrite and update it, or `remove` it first. Run `agent --link` after
installing to make the skill visible to agents.

## remove

Remove skills from the canonical directory (and any parked copy under
`disabled-skills`). Skills are selected by the slug — the name `list`
reports — matched case-insensitively.

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
| `name`        | The skill's slug — the frontmatter `name` slugified; the identity every command selects by. Directories whose manifest lacks a `name` are not skills and are never listed |
| `displayName` | The frontmatter `name` as declared — display-only (may contain spaces and mixed case)           |
| `description` | Skill description, collapsed onto a single line                                                 |
| `path`        | The skill's real on-disk directory (canonical dir when enabled, `disabled-skills` when not)      |
| `enabled`     | `true` in the canonical directory, `false` parked in `disabled-skills`                          |
| `installedAt` | Skill directory creation time as Unix seconds (UTC), or `null` when unavailable                 |

Use `agent --status` for each agent's link status.

## disable / enable

`disable` moves a skill's directory into `disabled-skills/`, hiding it from all
agents; `enable` moves it back. Files are preserved — lossless and reversible.
Skills are selected by the slug — the name `list` reports — matched
case-insensitively.

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
