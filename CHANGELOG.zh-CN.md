# 更新日志

本文件记录项目的所有显著变更。格式大致遵循
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/):处于 0.x 阶段时,破坏性变更可能出现在
次版本中(以 **(breaking)** 标注)。

英文版见 [CHANGELOG.md](CHANGELOG.md)。

## [0.18.0] — 2026-09-19

### 修复

- fix(add)：GitLab 的子路径安装现在可用。整仓归档会把所有内容包在
  `{repo}-{ref}` 目录里,而它被原样交给了发现逻辑,导致子路径解析多算了一层——
  所有 GitLab 子路径都以 "No valid skills found" 结束。现在所有取法都返回仓库根,
  解析不到内容的子路径也会直接报出名字（`Subpath "…" not found in …`），
  不再是含糊的失败。
- fix(add)：GitHub 对大仓库会截断 `git/trees` 列表,此时改为逐目录调用 `contents`
  API（GitHub 官方文档推荐的变通做法），而不是让安装失败。
- fix(add)：Git LFS 指针改为从 `media.githubusercontent.com` 取真实对象,不再把约
  130 字节的文本桩当成文件装上。
- fix(add)：可执行文件保留 `+x` 位。归档改用 `tar.gz` 下载（zip 会丢失 Unix 权限
  位），API 下载则按 `git/trees` 列表里的 mode 还原。
- fix(add)："已安装则跳过"的保护此前只用规范化后的目录名匹配。因此未规范化的停放副本
  （如 `pdf-master` 对应的 `disabled-skills/PDF Master`）会被漏掉，`add` 会为一个已
  安装的技能再造出第二份副本——同一个名字同时落进两个目录，随后还得靠 `enable` /
  `disable` 去收拾。
- fix(remove)：删除现在会清掉一个名字的所有副本——两个目录、两种拼写——而不再只按
  canonical 名字查找，因此 `remove --all` 不会再残留停放副本。也只在真的删掉东西时才把
  该名字计入已删除；此前删除失败会被静默计入成功。
- fix(add)：名字非 ASCII 的技能不再塌缩到共享的 `unnamed-skill` 槽位。槽位名折叠现在
  保留非 ASCII 字母数字（`中文技能`）以及文件名能容纳的字符（`c#`、`c++`），因此两个
  不同的技能名不会再落到同一个目录、也就不会让第二个被报成第一个的"已安装副本"。折叠后
  为空的名字（`"***"`）回退为原名的摘要（`skill-3f9a2c1d`）——同名稳定、异名不同。旧版本
  用旧占位名创建的目录保持原样：用 `remove unnamed-skill` 清理即可。
- fix(add)：槽位名现在按**字节**在字符边界处截断到 255 字节。此前按字符数截断，
  可能超出文件系统的 255 字节名字上限——255 个 CJK 字符就是 765 字节——导致安装以
  `ENAMETOOLONG` 失败。
- fix(cli)：无参数时打印的 banner 不再宣传 `update` 命令，该命令从未存在（运行它只会
  得到 `Unknown command: update`）。
- fix(package)：crates.io 包现在真正排除了 `AGENTS.md`；此前的 `exclude` 写的是
  `AGENT.md`（拼写错误），导致该文件仍被打进包中。

### 变更

- **(breaking)** 当 GitHub API 无法服务"收窄后的请求"（`subpath`，或
  `--skill` / `@skill`）时,`add` 不再回退去下载整仓归档。该回退会悄悄把下载范围
  放大到整个仓库,而最常见的两种失败——子路径或技能名写错——更是会先下完整仓再报
  同一个错。现在这些情况立即失败并给出 `Subpath "…" not found in …` 或
  `No skill named "…" in …`（附 `--list` 提示）；确实是 API 不可用时,则明确报错
  并把 `GITHUB_TOKEN` 作为未认证 60 次/小时限额的解法点出来。整仓安装、`--list`
  与 GitLab 仍然走归档——那是它们唯一的路径。
- `enable` / `disable` 现在对同时存在于两个目录的技能改为覆盖，而不是报错。被禁用的
  技能随时可能被第三方工具、或共享 canonical 目录的 agent 重新安装，因此同一个名字同时
  存在于 `skills/` 与 `disabled-skills/` 是正常状态，不是错误。被搬动的那份胜出：目标
  目录里已有的旧副本会被删除，除常规的 `Enabled`/`Disabled <name>` 外不再额外提示，因此
  一个技能名始终只对应一个目录。目录名仅规范化不同的副本（`PDF Master` 与
  `pdf-master`）视为同一技能，同样会被合并。此前这种 `enable` / `disable` 会以
  `Directory not empty (os error 66)` 失败，并留下那个重复副本。
- `remove` / `disable` / `enable` 的帮助文本与 `docs/CLI.md`（中英）不再宣称
  `-s '*'` 表示"全部技能"：只有 `add` 实现了该选择，其余三个命令用 `--all` 选择全部。
- deps：`zip` 2 → 8、`dirs` 6 → 7，并将 `noyalib` 固定为 `0.0.45`（此前的 `"0.0"`
  约束并不保证取到兼容的补丁版本）。

### 新增

- feat(list)：现在为每个技能报告其描述消耗的 token 估算量——`list --json` 中为
  `estimatedTokens`，纯文本输出每条显示 `~N tokens`，并汇总所有已启用技能常驻
  上下文的总量。描述是 harness 常驻上下文的部分（`SKILL.md` 正文只在技能触发时
  加载），因此这让已安装技能的固定成本变得可见。该数字用零依赖启发式估算（约 4 个
  ASCII 字符或 1 个非 ASCII 字符算 1 token），不是精确计数。
- feat(add)：`GITHUB_TOKEN` 现在也会发送给 `raw.githubusercontent.com` 与
  `media.githubusercontent.com`,因此私有仓库也能安装。
- perf(add)：文件改为小线程池并发下载（8 路），不再是逐个请求；`--skill` /
  `@skill` 的所有候选 `SKILL.md` 也改为一次批量抓取。

## [0.17.0] — 2026-09-19

### 移除

- **(breaking)** 项目级作用域。技能现在只存放在一个地方——规范目录
  `~/.agents/skills`，所有命令都只操作它。六个子命令的 `-p/--project <目录>` 旗标
  已移除，同时删除所有请求结构体上的 `global: bool` 字段、`AgentRequest.global`、
  `AgentOutcome.global`、`Agent.list()` 的参数，以及 `ListRequest`
  （`Manager::list` 现在不接收请求）。agent 表中的 `Agent.skills_dir` 已删除
  （`agents.jsonl` 84 行），`is_universal()` 与 `ensure_universal_agents()` 删除，
  `is_native` 现在只把解析出的技能目录与 `~/.agents/skills` 比较。项目级的
  `.misc/.gitignore` 技巧随之消失——`$HOME` 不进版本控制。`discover` 中的
  `AGENT_PROJECT_SKILL_DIRS` 与作用域无关（它是扫描**源仓库**时的容器目录列表），
  予以保留。`PathSpec::Cwd` 与基于 cwd 的检测规则同样保留：它们描述的是"agent
  装在哪"，而不是作用域。

### 变更

- **(breaking)** `add` 不再覆盖已安装的技能。被选中的技能若同名已安装（**无论
  启用还是禁用**），会记录在新的 `AddOutcome.skipped` 中并原样保留。因此本地改动
  永远不会被静默丢弃，对**已禁用**技能再安装也不会再产生重复副本（同一名字同时
  存在于 `skills/` 与 `disabled-skills/`）。替换已安装技能请用 `remove` + `add`
  —— 由于 `update` 已在 0.13.0 移除，这也正是更新技能的唯一方式。

### 修复

- (install) `disable`、`enable`、`remove` 现在能找到目录名未被规范化的技能。从
  agent 目录并入的技能会保留原目录名，它未必等于
  `sanitize_name(frontmatter name)`，而 `move_skill` / `get_canonical_path` /
  `remove` 会对其二次 sanitize：导致 `disable` 报 IO 错误、`remove` 报成功却
  什么都没删。

- (link) 并入时的同名冲突现在会在**两个**技能目录中、跨"原始名/规范化名"一并
  识别。此前 canonical 只按原始名检查，导致如 `pdf-master`（canonical）与
  `PDF Master`（agent）两个目录以同一技能名并存。

### 移除

- (install) `install_skill` 中的"替换 + 回滚"路径。既然 `add` 不再覆盖，目标目录
  不可能预先存在，`.old-*` 暂存目录及其回滚成为死代码。

- (cli) `main.rs` 中的 `project directory not found` 检查与 `explicit_project_dir`，
  以及 `Options: --project [dir], ...` 提示。

## [0.16.0] — 2026-09-19

### 新增

- `list` 现在报告每个技能的 `description` 与 `installedAt`。普通输出每个技能打印
  两行（名称 + 描述，随后是 `路径 [状态] · <本地时间>`）。

### 变更

- **(breaking)** `ListedSkill` 新增 `description: String` 与
  `installed_at: Option<u64>`（序列化为 `installedAt`）；`path` 的文档明确为
  "技能当前所在目录"——对已禁用技能是 `disabled-skills/<目录>`，而非规范目录。

### 说明

- `installedAt` 是**技能目录**的创建时间，是"技能落到磁盘的时间"的近似值，并非
  文件元数据：`add` 安装是精确的（暂存目录在安装时创建），但从 agent 目录并入的
  技能会保留该目录原本的创建时间，且对同名技能重新安装会刷新它。在不记录创建时间
  的文件系统（部分 Linux 文件系统）上为 `null`。

- `description` 已规整为单行，因此 YAML 块标量在普通输出与 `--json` 中都显示为
  一行。

### 依赖

- 新增 `jiff`，仅启用最小 feature（`std`、`tz-system`），用于按本地时区渲染
  `installedAt`。

## [0.15.0] — 2026-09-19

### 移除

- **(breaking)** 移除备份槽机制与 `--migrate` 旗标。`agent --link` 现在直接并入
  非空的技能目录，不再把它停放在 `.agents/backup-skills/<agent>/` 下：技能目录
  移入规范目录，非技能条目移入规范目录内的 `.misc/<agent>/`（点目录，安装/发现
  扫描不会把它误判为技能），同名冲突一律丢弃 agent 侧副本、保留已有副本——规范
  目录优先，已禁用（`disabled-skills`）的名字保持禁用、不被重新导入。指向规范
  目录的旧模型单技能符号链接同样丢弃，因为移入后它会变成自引用链接。因此链接是
  **单向**的：`agent --unlink` 只断开链接并重建空目录，已并入的内容留在规范目录，
  此后由 `remove`/`disable` 管理。

- **(breaking)** 库 API：移除 `AgentRequest.migrate`、`AgentStatus.pending_backup`
  与 `BackupStatus` 类型。`LinkOutcome::Migrated` 变体删除；`Linked` 的字段由
  `parked_skills`/`parked_others`/`backup_dir` 改为
  `adopted`/`quarantined`/`conflicts`；`Unlinked` 变为无字段变体（原
  `restored`/`restored_from` 字段删除）。

- **(breaking)** `agent --link` 不再因"存在未恢复的旧备份槽"而拒绝——该状态已不
  可能发生。拒绝现在仅保留一种情况：agent 技能目录是指向别处的符号链接。

### 说明

- 升级提示：遗留的 `.agents/backup-skills/` 目录已不再被读取。旧版本停放在其中的
  内容仍原样保留在磁盘上，请自行检查并清理。

- 项目级下隔离目录自带 `.misc/.gitignore`，让被隔离的文件不进版本控制（规范目录
  本身通常是需要提交的）。

## [0.14.0] — 2026-09-13

### 移除

- **(breaking)** 移除 `ListedSkill.scope` 和 `ListedSkill.agents`（库 API 与
  `list --json`），以及 `list -a/--agent` 旗标和 `ListRequest.agents`。agent
  可见性是作用域级状态——每个已链接或 native 的 agent 都能看到规范目录里的
  全部技能——per-skill 字段是冗余的，改由 `agent --status`
  （`linked || canonical`）推导。`ListedSkill` 现在只剩
  `name`/`path`/`enabled`。同时移除不再使用的 `Agent.hidden` 字段和
  `agent_display()` 辅助函数。

## [0.13.0] — 2026-09-13

### 移除

- **(breaking)** 彻底移除 lockfile 机制（`skills-lock.json` /
  `~/.agents/.skill-lock.json`）。`update` 移除后已无任何消费方，所有技能追踪
  现在完全基于目录扫描：`list` 和 `remove` 只看规范目录内容。已有 lockfile
  会被忽略，可自行删除。`list` 不再报告技能来源
  （`source`/`sourceUrl`/`sourceType` 字段已删）。同时移除不再使用的
  `sha2`、`icu_collator`、`icu_locale_core`、`walkdir` 依赖。

- **(breaking)** 移除 `update` 命令与 `Manager::update` API。原实现并不合理：
  忽略 lock 中记录的 `ref`（锁定分支/tag 时静默改用默认分支更新）、重装后从不
  回写 lockfile、无条件覆盖本地改动。需要最新版本请用 `add` 重新安装。

### 变更

- (install) Skill 安装改为原子操作。新内容先复制到目标旁边的 `.incoming-*` 暂存
  目录,再通过 rename 一次性换入:链接的 agent 要么看到完整的旧版,要么看到
  完整的新版;安装失败(磁盘满、权限等)时旧版本原样保留,不再出现删了一半的
  目录。目录扫描会跳过点开头的条目,中断残留的暂存目录不会被列成 skill。
  canonical 路径上若有同名文件占位,现在会被修复(替换)而不是安装失败。

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

[Unreleased]: https://github.com/skill-one/agents-skills/compare/v0.18.0...HEAD
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
