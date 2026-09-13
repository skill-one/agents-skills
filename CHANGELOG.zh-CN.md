# 更新日志

本文件记录项目的所有显著变更。格式大致遵循
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/):处于 0.x 阶段时,破坏性变更可能出现在
次版本中(以 **(breaking)** 标注)。

英文版见 [CHANGELOG.md](CHANGELOG.md)。

## [Unreleased](未发布)

### 移除

- **(breaking)** 移除 `update` 命令与 `Manager::update` API。原实现并不合理：
  忽略 lock 中记录的 `ref`（锁定分支/tag 时静默改用默认分支更新）、重装后从不
  回写 lockfile、无条件覆盖本地改动。需要最新版本请用 `add` 重新安装。

### 变更

- (install) Skill 安装改为原子操作。新内容先复制到目标旁边的 `.incoming-*` 暂存
  目录,再通过 rename 一次性换入:链接的 agent 要么看到完整的旧版,要么看到
  完整的新版;安装失败(磁盘满、权限等)时旧版本原样保留,不再出现删了一半的
  目录。目录扫描会跳过点开头的条目,中断残留的暂存目录不会被列成 skill。
  canonical 路径上若有同名文件占位,现在会被修复(替换)而不是安装失败。
- (lock) `computedHash` 现在与上游 skills.sh 的 hash 保持一致:对每个文件按
  `utf8(相对路径) + 0x00 + 文件字节 + 0x00` 追加进同一个 SHA-256 流,文件按
  大小写不敏感的路径顺序排序(ICU base 强度 collation,即
  `Intl.Collator("en", { sensitivity: "base" })`)。此前文件直接拼接无分隔符、
  按字节序排序。已有 lock 条目保留旧 hash 值,直到该 skill 被重新安装;hash
  仅作信息记录、不参与决策,因此无需迁移。

## [0.12.3] — 2026-09-12

### 新增

- (agents) 支持 WorkBuddy 国际版 WorkBuddy AI:项目级目录
  `.workbuddy-ai/skills`,全局目录 `~/.workbuddy-ai/skills`,通过
  `~/.workbuddy-ai` 或当前项目内的 `.workbuddy-ai` 检测安装。

## [0.12.2] — 2026-09-12

### 修复

- (link) agent 的 canonical/link 判定现在按作用域区分。项目级 universal
  (`.agents/skills`)但全局目录为厂商路径的 agent(如 Antigravity 的
  `~/.gemini/config/skills`、Codex 的 `~/.codex/skills`)此前在全局级也被
  当作"已链接"短路,导致装在 `~/.agents/skills` 的全局技能永远无法被 agent
  读取,`agent --status` 还会误报 `canonical`。现在全局级会建立真实的目录级
  符号链接(`--link` / `--unlink` / `--status` 均按作用域工作)。
- (link) 全局目录无法解析(env 变量未设置)时改为报告 skipped,不再报失败。

### 变更

- (库) 新增按作用域判定的 `is_native(agent, global, env)`;移除不再
  使用的 `universal_agents()`;`agent_skills_dir()` 对项目级 universal 的
  agent 也会解析其真实全局目录,而非返回 `None`。

## [0.12.1] — 2026-09-11

### 修复

- (agents) 将 Antigravity 的全局 skills 目录从 `.gemini/antigravity/skills`
  更正为 `.gemini/config/skills`——Antigravity 实际读取全局 skills 的位置。

## [0.12.0] — 2026-09-09

### 变更

- (库, breaking) `SkillsError::Yaml` 现在包装 [noyalib](https://crates.io/crates/noyalib)
  的错误类型(纯 Rust、零 unsafe、完整 serde 集成的 YAML 库),取代已停止维护的
  `serde_yaml`;SKILL.md frontmatter 改用 `noyalib` 解析。
- (依赖) `ureq` 2.x → 3.x(环境代理已内置,不再需要单独的 `proxy-from-env` feature)。

### 内部

- 将 `src/core/link.rs`(约 1600 行)与 `src/manager.rs`(约 1500 行)拆分为职责单一的
  子模块 —— `core/link/{mod,backup,outcome,path}` 与 `manager/{mod,types,select}`。
  行为无任何变化。
- 新增本更新日志(英文 + 中文),并在两份 README 中挂链接。

## [0.11.0] — 2026-09-06

### 新增

- perf(add):通过 GitHub API 只拉取所需的子目录(整包归档下载保留为回退路径)。

### 变更

- refactor(source):将 `WellKnown` 并入 `Download`;拒绝有歧义的 GitHub URL。
- (文档)重写两份 README 的来源格式章节。

## [0.10.1] — 2026-09-02

### 修复

- fix(link):迁移 agent 既有技能时保持已禁用技能的禁用状态。

## [0.10.0] — 2026-08-31

### 新增

- feat(agents):新增 Comate、JoyCode、LM Studio、QwenWork 及注册表补漏。

## [0.9.2] — 2026-08-30

### 变更

- refactor(core):将 agent 表抽取为声明式 `agents.jsonl`
  (新增 agent 现在只需改一行数据,无需动 Rust 代码)。
- (文档)AGENT.md 更名为 AGENTS.md。

## [0.9.1] — 2026-08-29

### 变更

- (文档)文档改写为英文,并附带 zh-CN 翻译版。
- (仓库)默认忽略点文件;反向包含仓库内的点条目。

## [0.9.0] — 2026-08-29

### 变更

- feat(cli) **(breaking)**:默认使用全局作用域;`--project <dir>` 进入项目作用域。

## [0.8.0] — 2026-08-29

### 新增

- feat(agent) **(breaking)**:链接时把 agent 既有技能目录整体"停车"进备份槽
  (unlink 时恢复)。

### 变更

- refactor(api) **(breaking)**:`core` 模块转为私有。
- refactor:精简 CLI 命令面;修复检测与更新的边界情况。
- refactor(core):去重子路径穿越检查;移除无用的字段。

### 修复

- fix(fetch):解压 gzip 下载内容,并且只嗅探一次归档类型。

## [0.7.0] — 2026-08-25

### 移除

- feat(api) **(breaking)**:移除 `Manager::add_source` 便捷方法。

## [0.6.0] — 2026-08-25

### 新增

- feat(agent):`--status` 对未链接 agent 也显示内部技能。
- feat(remove):同时扫描已禁用技能,使禁用中的技能也可移除。

## [0.5.0] — 2026-08-24

### 新增

- feat:技能的 enable/disable 命令。
- feat:快速、健壮的 GitHub 技能拉取;支持 `@skill` 过滤。

### 变更

- refactor **(breaking)**:库内 link API 更名为 agent 命名。
- refactor **(breaking)**:`link` 命令替换为 `agent` 子命令。
- (文档)文档拆分为 CLI/库/开发者三份指南。

## [0.4.0] — 2026-08-22

### 新增

- feat:完善链接状态、冲突迁移与 list 输出。

### 变更

- (文档)开发者指南迁移至 `docs/DEVELOPER.md`。

## [0.3.0] — 2026-08-22

### 变更

- refactor(link):`unlink` 与 `status` 合并进 `link` 命令。

## [0.2.0] — 2026-08-22

### 新增

- feat **(breaking)**:新增 `link`/`unlink` 命令;`add`/`remove` 保持仅操作规范目录。

## [0.1.0] — 2026-08-22

### 新增

- 首个发布:以库 + CLI 二进制双形态交付。
- fix:所有平台上拒绝 discover 子路径穿越。
- chore:升级 git2 至 0.21 以修复 RUSTSEC 安全通告。
- chore:双许可证、GitHub Actions、crates.io 发布元数据。

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
