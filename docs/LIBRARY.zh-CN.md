# agents-skills 库使用文档

简体中文 | [English](LIBRARY.md)

把技能管理能力嵌入自有 Rust 工具。CLI 用法见 [README](../README.zh-CN.md)，
完整命令参考见 [CLI.zh-CN.md](CLI.zh-CN.md)。

## 依赖引入

```toml
[dependencies]
agents-skills = "0.21"
```

## 快速开始

```rust
use agents_skills::{AddRequest, Manager};

let manager = Manager::new();
let outcome = manager.add(&AddRequest::new("anthropics/skills@pdf"))?;
println!("{} (skipped={})", outcome.skill.name, outcome.skipped);
let skills = manager.list()?;
```

## API 总览：[`Manager`]

每个方法接收一个 `Default + Clone` 的请求结构体，返回结构化结果。

| 方法                      | 请求               | 返回                                      |
| ------------------------- | ------------------ | ----------------------------------------- |
| [`Manager::add`]          | [`AddRequest`]     | [`AddOutcome`]（单个技能 + skipped 标记） |
| [`Manager::agent`]        | [`AgentRequest`]   | [`AgentOutcome`]（逐 agent 结果）         |
| [`Manager::agent_status`] | —                  | `Vec<`[`AgentStatus`]`>`                  |
| [`Manager::list`]         | —                  | `Vec<`[`ListedSkill`]`>`（可序列化）      |
| [`Manager::remove`]       | [`RemoveRequest`]  | [`RemoveOutcome`]（已移除名称）           |
| [`Manager::disable`]      | [`DisableRequest`] | [`DisableOutcome`]（已禁用名称）          |
| [`Manager::enable`]       | [`EnableRequest`]  | [`EnableOutcome`]（已启用名称）           |

请求字段：

| 结构体             | 字段                                                                                                 |
| ------------------ | ---------------------------------------------------------------------------------------------------- |
| [`AddRequest`]     | `source: String`（本地技能目录或 `owner/repo@<技能>`）、`reference: Option<String>`（分支/标签/SHA） |
| [`AgentRequest`]   | `agents: Vec<String>`（`"*"` 或名称，空 = 自动探测）、`unlink: bool`                                 |
| [`RemoveRequest`]  | `skills: Vec<String>`、`all: bool`                                                                   |
| [`DisableRequest`] | `skills: Vec<String>`、`all: bool`                                                                   |
| [`EnableRequest`]  | `skills: Vec<String>`、`all: bool`                                                                   |

要点：

- [`AddOutcome`] 描述单个技能：`source`、`skill`、`canonical_path`、`skipped`
  （同名已存在时为 `true`——`add` 绝不覆盖）。失败以 `Err` 返回。
  [`ListedSkill`] 字段与 `list --json` 一致（见 [CLI.zh-CN.md](CLI.zh-CN.md#list)）；
  技能目录可用 [`Manager::skill_dir`] 解析。
- CLI 的 `--link`/`--unlink`/`--status` 在库中拆为 [`Manager::agent`] 与
  [`Manager::agent_status`]——[`AgentRequest`] 只有 `unlink: bool`。并入语义
  （单向、规范目录优先）与 CLI 相同；仅当 agent 目录本身是指向别处的符号链接时
  返回 [`LinkOutcome::Refused`]。

### 常见操作

```rust
use agents_skills::{AddRequest, DisableRequest, EnableRequest, RemoveRequest};

// 用 reference 钉住分支/标签/SHA（None = 默认分支）
manager.add(&AddRequest {
    source: "anthropics/skills@pdf".into(),
    reference: Some("v1.2".into()),
    ..Default::default()
})?;

let skills = manager.list()?;
let json = serde_json::to_string_pretty(&skills)?; // 与 list --json 形状一致

manager.remove(&RemoveRequest  { skills: vec!["pdf".into()], ..Default::default() })?;
manager.disable(&DisableRequest{ skills: vec!["pdf".into()], ..Default::default() })?;
manager.enable(&EnableRequest  { skills: vec!["pdf".into()], ..Default::default() })?;
```

## 上下文注入：[`ManagerBuilder`]

```rust
let manager = Manager::builder()
    .home("/tmp/home")
    .config("/tmp/config")
    .cwd("/tmp/project")
    .env_var("CLAUDE_CONFIG_DIR", "/tmp/claude")
    .probe_system_dirs(false) // 跳过系统位置，保证测试封闭
    .build();
```

`Manager::new()` 等价于 `Manager::builder().build()`。可运行示例：

```bash
cargo run --example manage      # 在临时目录上演示 add → list → remove（无副作用）
cargo run --example add_skill   # 安装到真实环境
```

## 行为契约

库是纯数据的：从不打印、从不调用 `process::exit`；渲染与退出码由调用方决定。
无遥测——不会有任何数据离开本机。

[`Manager`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html
[`Manager::add`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.add
[`Manager::agent`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent
[`Manager::agent_status`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.agent_status
[`Manager::list`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.list
[`Manager::skill_dir`]: https://docs.rs/agents-skills/latest/agents_skills/struct.Manager.html#method.skill_dir
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
