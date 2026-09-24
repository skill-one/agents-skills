# Changelog

All notable changes to this project are documented in this file. The format
loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/): while in
0.x, breaking changes may land in minor releases (marked **(breaking)**).

For the Chinese version see [CHANGELOG.zh-CN.md](CHANGELOG.zh-CN.md).

## [0.23.0] — 2026-09-24

### Changed

- **(breaking)** Remote GitHub installs are now a single request: the whole
  repository tarball is downloaded from `codeload.github.com`, unpacked into a
  temp dir, and the named skill is matched locally. The GitHub REST API is
  never used, so the anonymous 60-requests-per-hour API rate limit no longer
  applies.
- **(breaking)** `GITHUB_TOKEN` is no longer read: only public repositories
  are supported.
- **(breaking)** Git LFS files install as their pointer stubs; LFS objects are
  no longer resolved through `media.githubusercontent.com`.
- **(breaking)** `--ref` no longer accepts abbreviated commit SHAs (use a
  branch, a tag, or a full 40-character SHA). Without `--ref` the repository's
  default branch (`HEAD`) is used.
- Removed the per-file parallel downloads, the recursive tree listing with its
  truncation fallback, and the ref-to-SHA resolution — all replaced by the
  single tarball request. The executable bit now comes from the tar archive's
  preserved permissions.
- Dependencies: added `tar` and `flate2` (pure Rust); removed `url`.

## Unreleased

## [0.22.0] — 2026-09-23

### Changed

- **(breaking)** A skill's identity is always its directory name. The
  `SKILL.md` frontmatter `name` field is no longer read or matched anywhere —
  local installs, `list`, and GitHub installs all use the directory basename.
  For `owner/repo@<skill>`, the repository tree is searched for a directory
  containing a `SKILL.md` whose basename matches `<skill>` case-insensitively
  (shallowest match wins), and only that directory is downloaded.
- A `SKILL.md` placed at the repository root is selected with the repository
  name (`owner/repo@<repo>`) and installs the whole repository.
- A missing or unparseable `SKILL.md` / frontmatter is no longer fatal: the
  description is then empty and the skill still installs.

## [0.21.0] — 2026-09-23

### Changed

- **(breaking)** `add` now installs exactly one explicitly named skill, and
  the source syntax is reduced to two forms: a local skill directory (it must
  directly contain a `SKILL.md`) and `owner/repo@<skill>` for one skill on
  GitHub — matched on the frontmatter name or the directory name,
  case-insensitively. A git ref is pinned with the new `--ref <branch|tag|SHA>`
  flag (library: `AddRequest.reference`) instead of a `/tree/<ref>` URL; without
  it the repository's default branch is used.
- **(breaking, library)** `AddRequest` now has only `source` and `reference`
  fields, and `AddOutcome` describes one skill (`source`, `skill`,
  `canonical_path`, `skipped: bool`). An install failure is returned as `Err`;
  the `InstallSuccess` / `InstallFailure` types are removed.
- Every remote install now goes through the GitHub API and downloads only the
  matched skill directory (the whole-repo archive path is gone).

### Removed

- **(breaking)** GitLab support (including self-hosted instances), SSH and
  generic `git clone` sources, direct HTTPS downloads (zip / tar archives and
  raw files), full GitHub URLs (`/tree/`, `/blob/`), repository subpath sources
  (`owner/repo/skills/pdf`), bare repository installs (`owner/repo`), the
  `--skill`/`-s` flag, the `--list`/`-l` flag, the `'*'` selector, and multiple
  source arguments on one `add` invocation. Each rejected form prints an error
  pointing at the supported syntax.
- The `git2`, `zip`, `tar`, and `flate2` dependencies and the `core::fetch`
  module (clone / download / unpack), along with repository-wide skill
  discovery (container directories, agent-directory scanning, full-tree
  fallback).

## [0.20.0] — 2026-09-22

### Changed

- **(breaking)** `ListedSkill.name` is now the skill's on-disk directory name
  (the identity `remove`/`disable`/`enable` use), not the `SKILL.md` frontmatter
  name — so `list` and `list --json` name a skill the same way the other
  commands do. The now-redundant `path` field is gone from `ListedSkill` and
  `list --json`; resolve a skill's directory with the new `Manager::skill_dir`.

## [0.19.0] — 2026-09-20

### Removed

- **(breaking)** the per-skill description token estimate added in 0.18.0.
  `estimatedTokens` is gone from `list --json`, the per-skill `~N tokens` figure
  and the enabled-skills total are gone from the plain output,
  `ListedSkill.estimated_tokens` is gone from the library API, and the
  dependency-free `core::tokens` heuristic module is deleted with it.

## [0.18.0] — 2026-09-19

### Fixed

- fix(add): a GitLab `subpath` install now works. A whole-repo archive wraps every
  entry in a `{repo}-{ref}` directory, which was handed to discovery as-is, so
  subpath resolution looked one level too deep — every GitLab subpath ended in
  "No valid skills found". Every fetch path now returns the repository root, and a
  subpath that resolves to nothing is named in the error
  (`Subpath "…" not found in …`) instead of ending as a generic failure.
- fix(add): a `git/trees` listing that GitHub truncates (large repositories) now
  falls back to per-directory `contents` listing — the workaround the GitHub docs
  recommend — instead of failing the install.
- fix(add): Git LFS pointers are fetched from `media.githubusercontent.com`
  instead of being installed as ~130-byte text stubs.
- fix(add): executable files keep their `+x` bit. Archives are downloaded as
  `tar.gz` (zip drops Unix modes) and API downloads restore the mode reported by
  the `git/trees` listing.
- fix(add): the "already installed → skip" guard only matched the normalized
  directory name. A parked copy under an unnormalized name
  (`disabled-skills/PDF Master` for `pdf-master`) was therefore missed, and `add`
  created a second copy of an already-installed skill — one name in both dirs,
  which `enable` / `disable` then had to resolve.
- fix(remove): removal now deletes _every_ copy of a name — both dirs, under
  either spelling — instead of looking up the canonical name only, so `remove
--all` can no longer leave a parked duplicate behind. A name is reported as
  removed only when something was actually deleted; a failed delete no longer
  counts as success.
- fix(add): a skill whose name is not ASCII no longer collapses onto a shared
  `unnamed-skill` slot. Slot-name folding keeps non-ASCII letters and digits
  (`中文技能`) and every character a file name can hold (`c#`, `c++`), so two
  differently named skills can no longer land on one directory and have the second
  reported as an already-installed copy of the first. A name the fold empties
  (`"***"`) falls back to a digest of the original name (`skill-3f9a2c1d`) — stable
  per name, distinct across names. A directory an older version created under the
  old placeholder is left as it is: `remove unnamed-skill` clears it.
- fix(add): a slot name is now truncated to 255 **bytes** at a character
  boundary. The previous character-based truncation could exceed the filesystem's
  255-byte name limit — 255 CJK characters are 765 bytes — and fail the install
  with `ENAMETOOLONG`.
- fix(cli): the banner printed when no arguments are given no longer advertises an
  `update` command, which never existed (running it printed
  `Unknown command: update`).
- fix(package): the crates.io package now really excludes `AGENTS.md`; the
  `exclude` list named `AGENT.md` (a typo), so the file was packaged anyway.

### Changed

- **(breaking)** `add` no longer falls back to a whole-repo archive when the GitHub
  API cannot serve a narrowed request (a `subpath`, or `--skill` / `@skill`). The
  fallback silently widened the download to the entire repository, and for the two
  commonest failures — a mistyped subpath or skill name — it downloaded everything
  only to report the same error. Those cases now fail immediately with
  `Subpath "…" not found in …` or `No skill named "…" in …` (plus a `--list` hint);
  a genuinely unavailable API reports the failure and names `GITHUB_TOKEN` as the
  remedy for the 60 requests/hour unauthenticated rate limit. A repository-wide
  install, `--list`, and GitLab keep using the archive — it is their only path.
- `enable` / `disable` now resolve a skill present in _both_ dirs by overwriting
  instead of failing. A disabled skill can be re-installed at any time by a
  third-party tool, or by an agent sharing the canonical dir, so one name living
  in both `skills/` and `disabled-skills/` is a normal state rather than an error.
  The copy being moved wins: the stale copy already in the target dir is deleted,
  with no notice beyond the usual `Enabled`/`Disabled <name>` line, so one name
  always maps to exactly one directory. Copies whose directory name differs only
  by normalization (`PDF Master` vs `pdf-master`) count as the same skill and are
  collapsed too. Previously such an `enable` / `disable` failed with
  `Directory not empty (os error 66)` and left the duplicate in place.
- `remove` / `disable` / `enable` no longer advertise `-s '*'` as "all skills" in
  their help text or in `docs/CLI.md` (both languages): only `add` implements that
  selection, and `--all` is how the other three select everything.
- deps: `zip` 2 → 8, `dirs` 6 → 7, and `noyalib` is pinned to `0.0.45` (the
  previous `"0.0"` requirement did not guarantee a compatible patch release).

### Added

- feat(list): each skill now reports the estimated token cost of its
  description — `estimatedTokens` in `list --json`, `~N tokens` per skill on the
  plain output, plus an always-on total for enabled skills. A description is
  what a harness keeps in context (the `SKILL.md` body only loads once the skill
  triggers), so this makes an installed skill's standing cost visible. The
  figure is a dependency-free heuristic (~4 ASCII characters, or 1 non-ASCII
  character, per token), not an exact count.
- feat(add): `GITHUB_TOKEN` is also sent to `raw.githubusercontent.com` and
  `media.githubusercontent.com`, so private repositories install too.
- perf(add): files are fetched with a small thread pool (8 in parallel) instead
  of one request at a time, and all candidate `SKILL.md` manifests for
  `--skill` / `@skill` are fetched in a single batch.

## [0.17.0] — 2026-09-19

### Removed

- **(breaking)** Project scope. Skills live in exactly one place now — the
  canonical dir `~/.agents/skills` — and every command operates on it. The
  `-p/--project <dir>` flag is gone from all six subcommands, along with the
  `global: bool` field on every request struct, `AgentRequest.global`,
  `AgentOutcome.global`, `Agent.list()`'s parameter, and `ListRequest`
  (`Manager::list` now takes no request). `Agent.skills_dir` is gone from the
  agent table (`agents.jsonl`, 84 lines), `is_universal()` and
  `ensure_universal_agents()` are gone, and `is_native` now only compares the
  resolved skills dir against `~/.agents/skills`. The project-scope
  `.misc/.gitignore` trick went away with it — `$HOME` is not version
  controlled. `discover`'s `AGENT_PROJECT_SKILL_DIRS` is unrelated (it lists
  container dirs to scan inside a _source_ repository) and stays.
  `PathSpec::Cwd` and cwd-based detection rules stay too: they describe where an
  agent is installed, not a scope.

### Changed

- **(breaking)** `add` no longer overwrites an installed skill. A selected skill
  whose name is already installed — enabled _or_ disabled — is reported in the
  new `AddOutcome.skipped` and left untouched. Local edits are therefore never
  silently discarded, and installing over a _disabled_ skill can no longer leave
  a duplicate copy behind (one name in both `skills/` and `disabled-skills/`).
  Replace an installed skill with `remove` + `add` — that is also how it is
  updated now, since `update` was removed in 0.13.0.

### Fixed

- (install) `disable`, `enable` and `remove` now find skills whose directory
  name was never normalized. A skill adopted from an agent dir keeps its
  original directory name, which need not equal
  `sanitize_name(frontmatter name)`, while `move_skill` / `get_canonical_path` /
  `remove` re-sanitized it: `disable` failed with an IO error and `remove`
  reported success without deleting anything.

- (link) Name clashes on adopt are detected across normalized and unnormalized
  names in _both_ skills dirs. The canonical dir was only checked with the raw
  name, so e.g. `pdf-master` (canonical) plus `PDF Master` (agent) both ended up
  installed under the same skill name.

### Removed

- (install) The replace-and-rollback path in `install_skill`. With `add` never
  overwriting, the destination can no longer pre-exist, so the `.old-*` staging
  dir and its rollback were dead code.

- (cli) The `project directory not found` check and `explicit_project_dir` in
  `main.rs`, and the `Options: --project [dir], ...` hints.

## [0.16.0] — 2026-09-19

### Added

- `list` now reports each skill's `description` and `installedAt`. The plain
  output prints two lines per skill (name + description, then
  `path [status] · <local time>`).

### Changed

- **(breaking)** `ListedSkill` gained `description: String` and
  `installed_at: Option<u64>` (serialized as `installedAt`), and its `path` is
  documented as "the directory the skill currently lives in" — for a disabled
  skill that is `disabled-skills/<dir>`, not the canonical dir.

### Notes

- `installedAt` is the **skill directory's** creation time, an approximation of
  when the skill landed on disk, and is not read from file metadata: it is exact
  for `add` installs (the staged directory is created at install time), but a
  skill adopted from an agent dir keeps that directory's original creation time,
  and re-installing over a skill refreshes it. It is `null` where the filesystem
  records no creation time (some Linux filesystems).

- `description` is collapsed onto a single line, so a YAML block scalar renders
  as one line in both the plain output and `--json`.

### Dependencies

- Added `jiff` with minimal features (`std`, `tz-system`) to render
  `installedAt` in the local time zone.

## [0.15.0] — 2026-09-19

### Removed

- **(breaking)** The backup-slot mechanism and the `--migrate` flag. `agent
--link` now adopts a non-empty skills directory outright instead of parking it
  under `.agents/backup-skills/<agent>/`: skill directories are moved into the
  canonical dir, non-skill entries into `.misc/<agent>/` inside it (a dot-dir,
  so install/discovery scans never mistake them for skills), and name clashes
  are dropped in favour of the existing copy — the canonical copy wins, and a
  name disabled in `disabled-skills` stays disabled rather than being
  re-imported. Legacy per-skill symlinks into the canonical dir are dropped as
  well, since moving them in would have made them self-referential. Linking is
  therefore **one-way**: `agent --unlink` only disconnects the agent and
  recreates an empty dir; adopted content stays in the canonical dir and is
  managed by `remove`/`disable` from then on.

- **(breaking)** Library API: `AgentRequest.migrate`,
  `AgentStatus.pending_backup` and the `BackupStatus` type are gone.
  `LinkOutcome::Migrated` is removed; `Linked` now carries
  `adopted`/`quarantined`/`conflicts` instead of
  `parked_skills`/`parked_others`/`backup_dir`, and `Unlinked` is a unit variant
  (its `restored`/`restored_from` fields are gone).

- **(breaking)** `agent --link` no longer refuses on a stale backup slot — that
  state can no longer occur. Refusal is now reserved for an agent skills dir
  that is a symlink pointing elsewhere.

### Notes

- Upgrading: a leftover `.agents/backup-skills/` directory is no longer read by
  the tool. Content parked there by an earlier version is untouched on disk;
  inspect and remove it manually.

- In project scope the quarantine dir carries its own `.misc/.gitignore` so
  quarantined files stay out of version control (the canonical dir itself is
  normally committed).

## [0.14.0] — 2026-09-13

### Removed

- **(breaking)** `ListedSkill.scope` and `ListedSkill.agents` (library API and
  `list --json`), together with the `list -a/--agent` flag and
  `ListRequest.agents`. Agent visibility is scope-level state — every linked
  or native agent sees all skills in the canonical dir — so a per-skill field
  was redundant; derive it from `agent --status` (`linked || canonical`)
  instead. `ListedSkill` now carries `name`/`path`/`enabled` only. The
  unused `Agent.hidden` flag and `agent_display()` helper are also dropped.

## [0.13.0] — 2026-09-13

### Removed

- **(breaking)** The lockfile mechanism entirely (`skills-lock.json` /
  `~/.agents/.skill-lock.json`). After the `update` removal nothing consumed
  the recorded metadata, so all skill tracking is directory-scan based now:
  `list` and `remove` rely purely on the canonical dir contents. Existing
  lockfiles are simply ignored and can be deleted. `list` no longer reports a
  skill's origin (`source`/`sourceUrl`/`sourceType` fields removed). The now
  unused `sha2`, `icu_collator`, `icu_locale_core` and `walkdir` dependencies
  are dropped.

- **(breaking)** The `update` command and `Manager::update` API. The
  implementation was unsound: it ignored the recorded `ref` (so pinned
  branches/tags silently updated from the default branch), never refreshed the
  lockfile after reinstalling, and unconditionally overwrote local changes.
  Reinstall with `add` to get the latest version.

### Changed

- (install) Skills are now installed atomically. The new content is copied into
  a `.incoming-*` staging dir next to the destination and swapped in with
  renames, so linked agents only ever see a complete skill version, and a
  failed install (disk full, permissions, ...) leaves the previous version
  untouched instead of a half-deleted directory. Directory scans skip dot-prefixed
  entries, so interrupted staging leftovers can never be listed as skills.
  A stray file at the skill's canonical path is now repaired (replaced) instead
  of failing the install.

## [0.12.3] — 2026-09-12

### Added

- (agents) Support WorkBuddy AI, the international edition of WorkBuddy:
  project dir `.workbuddy-ai/skills`, global dir `~/.workbuddy-ai/skills`,
  detected via `~/.workbuddy-ai` or `.workbuddy-ai` in the current project.

## [0.12.2] — 2026-09-12

### Fixed

- (link) Agent canonical/link detection is now scope-aware. Agents universal at
  project scope (`.agents/skills`) but with a vendor-specific global dir
  (e.g. Antigravity's `~/.gemini/config/skills`, Codex's `~/.codex/skills`)
  were treated as "already linked" at global scope too, so global installs in
  `~/.agents/skills` never reached the agent and `agent --status` falsely
  reported `canonical`. They now get a real directory-level symlink at
  global scope (`--link` / `--unlink` / `--status` all honor scope).
- (link) An agent whose global dir cannot be resolved (unset env var) is
  now reported as skipped instead of a failure.

### Changed

- (library) Added scope-aware `is_native(agent, global, env)`; removed the
  now-unused `universal_agents()` helper; `agent_skills_dir()` resolves
  the agent's real global dir instead of returning `None` for
  project-universal agents.

## [0.12.1] — 2026-09-11

### Fixed

- (agents) Corrected the Antigravity global skills directory from
  `.gemini/antigravity/skills` to `.gemini/config/skills`, where Antigravity
  actually reads global skills from.

## [0.12.0] — 2026-09-09

### Changed

- (library, breaking) `SkillsError::Yaml` now wraps `noyalib::Error` instead of
  the archived `serde_yaml::Error`; SKILL.md frontmatter is parsed with
  [noyalib](https://crates.io/crates/noyalib) (a pure-Rust YAML library with
  zero unsafe code and full serde integration).
- (deps) Upgraded `ureq` 2.x → 3.x (environment proxies are built in, so the
  separate `proxy-from-env` feature is gone).

### Internal

- Split `src/core/link.rs` (~1600 lines) and `src/manager.rs` (~1500 lines)
  into focused submodules — `core/link/{mod,backup,outcome,path}` and
  `manager/{mod,types,select}`. No behavior change.
- Added this changelog (English + zh-CN) and linked it from both READMEs.

## [0.11.0] — 2026-09-06

### Added

- perf(add): fetch only the needed subdirectory via the GitHub API (the full
  archive download remains the fallback).

### Changed

- refactor(source): merged `WellKnown` into `Download`; ambiguous GitHub URLs
  are rejected.
- (docs) reworked the source-formats section in both READMEs.

## [0.10.1] — 2026-09-02

### Fixed

- fix(link): keep disabled skills disabled when migrating agent skills.

## [0.10.0] — 2026-08-31

### Added

- feat(agents): Comate, JoyCode, LM Studio, QwenWork and registry gaps.

## [0.9.2] — 2026-08-30

### Changed

- refactor(core): extract the agent table into declarative `agents.jsonl`
  (adding an agent is now a one-line data edit, no Rust changes).
- (docs) renamed AGENT.md to AGENTS.md.

## [0.9.1] — 2026-08-29

### Changed

- (docs) rewrote the docs in English with zh-CN translations alongside.
- (repo) ignore dotfiles by default; re-include repo dot-entries.

## [0.9.0] — 2026-08-29

### Changed

- feat(cli) **(breaking)**: default to global scope; `--project <dir>` opts
  into project scope.

## [0.8.0] — 2026-08-29

### Added

- feat(agent) **(breaking)**: park pre-existing skills dirs into backup slots
  when linking (unlink restores them).

### Changed

- refactor(api) **(breaking)**: made the `core` module private.
- refactor: trimmed the CLI surface; fixed detection and update edge cases.
- refactor(core): deduped the subpath traversal check; dropped a dead field.

### Fixed

- fix(fetch): decompress gzip downloads and sniff the archive kind once.

## [0.7.0] — 2026-08-25

### Removed

- feat(api) **(breaking)**: removed the `Manager::add_source` convenience
  method.

## [0.6.0] — 2026-08-25

### Added

- feat(agent): show internal skills for unlinked agents in `--status`.
- feat(remove): also scan disabled skills so disabled skills can be removed.

## [0.5.0] — 2026-08-24

### Added

- feat: enable/disable commands for skills.
- feat: fast, robust GitHub skill fetching; honor the `@skill` filter.

### Changed

- refactor **(breaking)**: renamed the library link API to agent naming.
- refactor **(breaking)**: replaced the `link` command with the `agent`
  subcommand.
- (docs) split the documentation into CLI/library/developer guides.

## [0.4.0] — 2026-08-22

### Added

- feat: refined link status, conflict migration, and list output.

### Changed

- (docs) moved the developer guide to `docs/DEVELOPER.md`.

## [0.3.0] — 2026-08-22

### Changed

- refactor(link): merged `unlink` and `status` into the `link` command.

## [0.2.0] — 2026-08-22

### Added

- feat **(breaking)**: `link`/`unlink` commands; `add`/`remove` stay
  canonical-only.

## [0.1.0] — 2026-08-22

### Added

- Initial release: shipped as a library alongside the CLI binary.
- fix: reject path traversal in discover subpaths on all platforms.
- chore: upgrade git2 to 0.21 to fix RUSTSEC advisories.
- chore: dual license, GitHub Actions, crates.io release metadata.

[Unreleased]: https://github.com/skill-one/agents-skills/compare/v0.22.0...HEAD
[0.22.0]: https://github.com/skill-one/agents-skills/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/skill-one/agents-skills/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/skill-one/agents-skills/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/skill-one/agents-skills/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/skill-one/agents-skills/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/skill-one/agents-skills/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/skill-one/agents-skills/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/skill-one/agents-skills/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/skill-one/agents-skills/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/skill-one/agents-skills/compare/v0.12.3...v0.13.0
[0.12.3]: https://github.com/skill-one/agents-skills/compare/v0.12.2...v0.12.3
[0.12.2]: https://github.com/skill-one/agents-skills/compare/v0.12.1...v0.12.2
[0.12.1]: https://github.com/skill-one/agents-skills/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/skill-one/agents-skills/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/skill-one/agents-skills/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/skill-one/agents-skills/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/skill-one/agents-skills/compare/v0.9.2...v0.10.0
[0.9.2]: https://github.com/skill-one/agents-skills/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/skill-one/agents-skills/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/skill-one/agents-skills/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/skill-one/agents-skills/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/skill-one/agents-skills/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/skill-one/agents-skills/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/skill-one/agents-skills/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/skill-one/agents-skills/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/skill-one/agents-skills/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/skill-one/agents-skills/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/skill-one/agents-skills/releases/tag/v0.1.0
