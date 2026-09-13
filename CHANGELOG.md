# Changelog

All notable changes to this project are documented in this file. The format
loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/): while in
0.x, breaking changes may land in minor releases (marked **(breaking)**).

For the Chinese version see [CHANGELOG.zh-CN.md](CHANGELOG.zh-CN.md).

## [Unreleased]

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

[Unreleased]: https://github.com/skill-one/agents-skills/compare/v0.12.3...HEAD
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
