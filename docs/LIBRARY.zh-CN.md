# agents-skills 库使用文档

简体中文 | [English](LIBRARY.md)

面向**库使用者**：把技能管理能力嵌入自有 Rust 工具。CLI 用法见 [README](../README.zh-CN.md)，命令行参考见 [CLI.zh-CN.md](CLI.zh-CN.md)。

## 依赖引入

```toml
[dependencies]
agents-skills = "0.10"
```

## 快速开始

```rust
use agents_skills::{AddRequest, AgentRequest, Manager};

fn main() -> agents_skills::Result<()> {
    let manager = Manager::builder().build(); // 等价于 Manager::new()

    manager.agent(&AgentRequest::default())?;        // 链接所有已安装 agent
    let outcome = manager.add(&AddRequest::new("anthropics/skills"))?; // 安装技能包
    println!("installed {} skill(s)", outcome.installed.len());

    // agent_status 列出每个 agent 的链接状态；未链接且自带内容的 agent
    // 会分类暴露私有的技能与其他文件，以及待恢复的备份槽。
    for s in manager.agent_status(false) {
        println!("{}: linked={}", s.name, s.linked);
        if !s.internal_skills.is_empty() {
            println!("  skills: {}", s.internal_skills.join(", "));
        }
        if !s.internal_others.is_empty() {
            println!("  others: {}", s.internal_others.join(", "));
        }
        if let Some(b) = &s.pending_backup {
            println!("  backup at {}: {}", b.path.display(), b.items.join(", "));
        }
    }
    Ok(())
}
```

## 高层 API：[`Manager`]

每个方法接收一个纯数据请求结构体，返回结构化结果；请求结构体均为
`Default + Clone`，可用字段覆盖构建。

| 方法                      | 请求               | 返回                                   |
| ------------------------- | ------------------ | -------------------------------------- |
| [`Manager::add`]          | [`AddRequest`]     | [`AddOutcome`]（已安装 + 链接 + 失败） |
| [`Manager::agent`]        | [`AgentRequest`]   | [`AgentOutcome`]（逐 agent 结果）      |
| [`Manager::agent_status`] | `bool`（global）   | `Vec<`[`AgentStatus`]`>`               |
| [`Manager::list`]         | [`ListRequest`]    | `Vec<`[`ListedSkill`]`>`（可序列化）   |
| [`Manager::remove`]       | [`RemoveRequest`]  | [`RemoveOutcome`]（已移除名称）        |
| [`Manager::disable`]      | [`DisableRequest`] | [`DisableOutcome`]（已禁用名称）       |
| [`Manager::enable`]       | [`EnableRequest`]  | [`EnableOutcome`]（已启用名称）        |

### 请求结构体字段

| 结构体             | 字段（除 `global: bool` 外）                                                                                |
| ------------------ | ----------------------------------------------------------------------------------------------------------- |
| [`AddRequest`]     | `source: String`、`skills: Vec<String>`（`"*"` 或具体名，空 = 全部）、`list_only: bool`                     |
| [`AgentRequest`]   | `agents: Vec<String>`、`unlink: bool`、`migrate: bool`                                                      |
| [`ListRequest`]    | —（作用域仅由 `global: bool` 决定）                                                                         |
| [`RemoveRequest`]  | `skills: Vec<String>`、`all: bool`                                                                          |
| [`DisableRequest`] | `skills: Vec<String>`、`all: bool`                                                                          |
| [`EnableRequest`]  | `skills: Vec<String>`、`all: bool`                                                                          |

所有请求结构体带 `global: bool` 字段：`true` 操作全局 `~/.agents/skills`（CLI 默认），
`false` 操作项目级 `./.agents/skills`（对应 CLI 的 `--project`）。项目根取自
`Env.cwd`（可用 `Manager::builder().cwd()` 覆盖，对应 CLI 的 `--project <目录>`）。
`AgentRequest` 的 `agents` 字段用于限定 agent（`"*"` 或具体名，
空 = 自动探测）。哪些 agent 能看到某个技能是作用域级状态（由
[`Manager::agent_status`] 查询），不是 per-skill 属性。

### 与 CLI 的对应约定

- **`add` 单 source**：CLI 的 `add <source...>` 可一次装多个源，库的
  [`AddRequest`] 只接受单个 `source: String`。要装多个源请多次调用
  `manager.add(...)`，每次返回独立的 [`AddOutcome`]。
- **`AgentRequest` 的 link 约定**：CLI 的 `agent` 命令 `--link`/`--unlink`/`--status`
  三选一互斥；库把 `--status` 拆为独立的 [`Manager::agent_status`]，因此
  [`AgentRequest`] 只需区分 link 与 unlink：`unlink: false`（默认）即 link，
  `unlink: true` 即 unlink，`migrate: true` 仅在 link 时生效（对应 CLI
  `--link --migrate`）。链接从不销毁已有内容：非空技能目录整体移入备份槽
  `.agents/backup-skills/<agent>/skills/`，unlink 时一次 rename 恢复；`migrate: true`
  把其中的技能移入规范目录（同名时规范目录副本优先；已禁用的技能保持禁用，
  agent 侧副本留在备份槽）。仅当 agent 目录是指向别处的符号链接，或存在未恢复
  的旧备份时报 [`LinkOutcome::Refused`]。

### 常见操作

```rust
use agents_skills::{AddRequest, DisableRequest, EnableRequest, ListRequest, RemoveRequest};

// 安装指定技能 / 只列出不安装
let outcome = manager.add(&AddRequest {
    source: "anthropics/skills".into(),
    skills: vec!["pdf".into()],   // 省略则安装全部
    list_only: false,             // true 则只列出可用技能
    ..Default::default()
})?;

// 列出技能（global 字段选作用域；--json 为 CLI 对应能力）
let skills = manager.list(&ListRequest::default())?;
let json = serde_json::to_string_pretty(&skills)?; // CLI 的 list --json

// 移除技能
manager.remove(&RemoveRequest { skills: vec!["pdf".into()], ..Default::default() })?;

// 禁用 / 启用（把技能目录移出 / 移回规范目录）
manager.disable(&DisableRequest { skills: vec!["pdf".into()], ..Default::default() })?;
manager.enable(&EnableRequest { skills: vec!["pdf".into()], ..Default::default() })?;
```

## 上下文注入：[`ManagerBuilder`]

```rust
let manager = Manager::builder()
    .home("/tmp/home")
    .config("/tmp/config")
    .cwd("/tmp/project")
    .env_var("CLAUDE_CONFIG_DIR", "/tmp/claude")
    .build();
```

用于沙箱/测试，避免触碰真实环境；`Manager::new()` 等价于 `Manager::builder().build()`。
沙箱中再加 `.probe_system_dirs(false)` 可让 agent 探测完全不读取系统位置
（如 `/Applications/ZCode.app`），保证结果封闭可复现。

## 示例

```bash
cargo run --example manage      # 在临时目录上演示 add → list → remove（无副作用）
cargo run --example add_skill   # 通过 Manager 安装到真实环境
```

## 行为契约

库保持**纯数据**：从不打印、从不调用 `process::exit`，结果结构化，错误通过 `Result`
上抛；渲染与退出码由调用方决定。库**无遥测**——不会有任何数据离开你的机器。

[`Manager`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html
[`Manager::add`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.add
[`Manager::agent`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent
[`Manager::agent_status`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent_status
[`Manager::list`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.list
[`Manager::remove`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.remove
[`Manager::disable`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.disable
[`Manager::enable`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.enable
[`ManagerBuilder`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ManagerBuilder.html
[`AddRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AddRequest.html
[`AddOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AddOutcome.html
[`AgentRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentRequest.html
[`AgentOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentOutcome.html
[`AgentStatus`]: https://docs.rs/agents-skills/latest/agents_skills/struct.AgentStatus.html
[`LinkOutcome::Refused`]: https://docs.rs/agents-skills/latest/agents_skills/enum.LinkOutcome.html
[`ListRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListRequest.html
[`ListedSkill`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListedSkill.html
[`RemoveRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveRequest.html
[`RemoveOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveOutcome.html
[`DisableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableRequest.html
[`DisableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableOutcome.html
[`EnableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableRequest.html
[`EnableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableOutcome.html
