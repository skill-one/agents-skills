# Changelog

All notable changes to this project are documented in this file. The format
loosely follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/): while in
0.x, breaking changes may land in minor releases (marked **(breaking)**).

For the Chinese version see [CHANGELOG.zh-CN.md](CHANGELOG.zh-CN.md).

## [Unreleased]

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

[Unreleased]: https://github.com/skill-one/agents-skills/compare/v0.12.0...HEAD
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
