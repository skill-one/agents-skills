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
    // 会分类暴露私有的技能与其他文件，即链接时会被并入规范目录的内容。
    for s in manager.agent_status(false) {
        println!("{}: linked={}", s.name, s.linked);
        if !s.internal_skills.is_empty() {
            println!("  skills: {}", s.internal_skills.join(", "));
        }
        if !s.internal_others.is_empty() {
            println!("  others: {}", s.internal_others.join(", "));
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
| [`Manager::add`]          | [`AddRequest`]     | [`AddOutcome`]（已安装 + 跳过 + 失败） |
| [`Manager::agent`]        | [`AgentRequest`]   | [`AgentOutcome`]（逐 agent 结果）      |
| [`Manager::agent_status`] | —                  | `Vec<`[`AgentStatus`]`>`               |
| [`Manager::list`]         | —                  | `Vec<`[`ListedSkill`]`>`（可序列化）   |
| [`Manager::remove`]       | [`RemoveRequest`]  | [`RemoveOutcome`]（已移除名称）        |
| [`Manager::disable`]      | [`DisableRequest`] | [`DisableOutcome`]（已禁用名称）       |
| [`Manager::enable`]       | [`EnableRequest`]  | [`EnableOutcome`]（已启用名称）        |

### 请求结构体字段

| 结构体             | 字段                                                                                             |
| ------------------ | ------------------------------------------------------------------------------------------------ |
| [`AddRequest`]     | `source: String`、`skills: Vec<String>`（`"*"` 或具体名，空 = 全部）、`list_only: bool`          |
| [`AgentRequest`]   | `agents: Vec<String>`、`unlink: bool`                                                            |
| [`RemoveRequest`]  | `skills: Vec<String>`、`all: bool`                                                               |
| [`DisableRequest`] | `skills: Vec<String>`、`all: bool`                                                               |
| [`EnableRequest`]  | `skills: Vec<String>`、`all: bool`                                                               |

所有命令只操作规范目录 `~/.agents/skills`，没有作用域选项。
`AgentRequest` 的 `agents` 字段用于限定 agent（`"*"` 或具体名，空 = 自动探测）。
哪些 agent 能看到某个技能不是 per-skill 属性——每个已链接或 native 的 agent 都能
看到整个规范目录，由 [`Manager::agent_status`] 查询。

### 结果结构体字段

[`Manager::list`] 返回 [`ListedSkill`]，即 `list --json` 序列化的精确形状：
`name`、`description`（已规整为单行）、`path`（技能当前所在目录：规范目录或
`disabled-skills`）、`enabled`、`installed_at`（Unix 秒，UTC；文件系统不记录
创建时间时为 `None`）。`installed_at` 是"技能落到磁盘的时间"的近似值：`add`
安装是精确的，但从 agent 目录并入的技能会保留该目录原本的创建时间。

[`Manager::add`] 返回 [`AddOutcome`]：`skills`（全部发现的技能）、`selected`、
`installed`、`skipped`（同名已安装——`add` 绝不覆盖）、`failed`。

### 与 CLI 的对应约定

- **`add` 单 source**：CLI 的 `add <source...>` 可一次装多个源，库的
  [`AddRequest`] 只接受单个 `source: String`。要装多个源请多次调用
  `manager.add(...)`，每次返回独立的 [`AddOutcome`]。
- **`AgentRequest` 的 link 约定**：CLI 的 `agent` 命令 `--link`/`--unlink`/`--status`
  三选一互斥；库把 `--status` 拆为独立的 [`Manager::agent_status`]，因此
  [`AgentRequest`] 只需区分 link 与 unlink：`unlink: false`（默认）即 link，
  `unlink: true` 即 unlink。链接会把 agent 技能目录中的存量内容并入规范目录，
  且是单向的：技能目录移入规范目录，非技能条目移入规范目录内的 `.misc/<agent>/`，
  同名冲突一律丢弃 agent 侧副本、保留已有副本（规范目录优先；已禁用
  `disabled-skills` 的技能保持禁用、不被重新导入）。unlink 不会把已并入内容移回。
  仅当 agent 目录是指向别处的符号链接时报 [`LinkOutcome::Refused`]。

### 常见操作

```rust
use agents_skills::{AddRequest, DisableRequest, EnableRequest, RemoveRequest};

// 安装指定技能 / 只列出不安装
let outcome = manager.add(&AddRequest {
    source: "anthropics/skills".into(),
    skills: vec!["pdf".into()],   // 省略则安装全部
    list_only: false,             // true 则只列出可用技能
    ..Default::default()
})?;

// 列出技能（--json 为 CLI 对应能力）
let skills = manager.list()?;
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
[`ListedSkill`]: https://docs.rs/agents-skills/latest/agents_skills/struct.ListedSkill.html
[`RemoveRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveRequest.html
[`RemoveOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.RemoveOutcome.html
[`DisableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableRequest.html
[`DisableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.DisableOutcome.html
[`EnableRequest`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableRequest.html
[`EnableOutcome`]: https://docs.rs/agents-skills/latest/agents_skills/struct.EnableOutcome.html
