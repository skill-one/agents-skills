# agents-skills 开发者文档

简体中文 | [English](DEVELOPER.md)

面向**本项目开发者**：项目结构、开发流程、测试与发布。功能概览与 CLI 用法见
[README](../README.zh-CN.md)，命令行参考见 [CLI.zh-CN.md](CLI.zh-CN.md)，库使用者见
[LIBRARY.zh-CN.md](LIBRARY.zh-CN.md)。

## 架构分层

项目刻意分层，库与 CLI 职责严格分离：

- **库**（`src/lib.rs` + `src/manager/` + `src/core/`）—— 纯数据：从不打印、
  从不调用 `process::exit`，错误通过 `Result` 上抛。
- **CLI**（`src/main.rs` + `src/cli.rs` + `src/commands/`）—— 库之上的薄渲染层：
  只负责 clap 参数拆解、把请求结构体交给 `Manager`、再把结果渲染成人类/机器可读
  输出并决定退出码。

每个 CLI 命令对应一个 `Manager` 方法，CLI 的 flag 对应请求结构体字段。新增能力时
应先在 `core`/`Manager` 层实现，再在 CLI 层渲染；不要让 CLI 层直接碰领域逻辑。

## 项目结构

```
src/
├── lib.rs              库根：Manager 门面 + 请求/结果类型 + 私有 core 模块
├── manager/            高层 Manager 门面（add/list/remove/disable/enable/link）
│   ├── mod.rs          Manager 方法（每个 CLI 命令对应一个）
│   ├── types.rs        与 CLI 层共享的请求/结果结构体
│   ├── select.rs       选择辅助函数（技能匹配 + agent 解析）
│   └── tests.rs        选择辅助函数的单元测试
├── error.rs            统一错误类型与 Result 别名
├── core/               领域逻辑（纯函数、依赖可注入）
│   ├── mod.rs          模块组织与重导出
│   ├── source.rs       来源字符串解析（本地目录或 `owner/repo@<技能>`）
│   ├── agents.rs       agent 表的声明式解释器(目录解析 + 安装检测)
│   ├── agents.jsonl    agent 表:每个 agent 一行 JSON
│   ├── discover.rs     技能发现：name = 目录名，描述尽力读取
│   ├── github.rs       GitHub tarball 拉取（codeload 单请求 + 本地目录匹配）
│   ├── install.rs      安装技能到规范目录 + 已装清单
│   ├── link/           目录级 agent 链接（link/unlink）
│   │   ├── mod.rs      链接编排 + 存量内容并入
│   │   ├── outcome.rs  LinkOutcome 结果枚举
│   │   ├── path.rs     路径分类辅助函数
│   │   └── tests.rs    链接机制的单元测试
│   └── test_utils.rs   单元测试共享夹具
├── main.rs             bin 入口（库之上的薄 CLI）
├── cli.rs              clap 命令树（命令、flags，不设别名）
└── commands/           CLI 渲染层（仅参数拆解 + 输出）
    ├── mod.rs
    ├── add.rs
    ├── remove.rs
    ├── list.rs
    ├── disable.rs
    ├── enable.rs
    └── agent.rs

examples/
├── add_skill.rs        通过 Manager 门面安装技能（真实用法）
└── manage.rs           在临时目录上演示 add → list → remove 生命周期

tests/
├── common/mod.rs       集成测试共享夹具
├── lib_api.rs          库 API 集成测试
├── cli_add.rs
├── cli_remove.rs
├── cli_list.rs
├── cli_agent.rs
├── cli_enable_disable.rs
└── cli_version.rs
```

## 新增 agent

agent 表位于 `src/core/agents.jsonl` —— 每个 agent 一行 JSON,编译期通过
`include_str!` 嵌入二进制。新增、修改或删除 agent 只需编辑该文件的一行,
无需改动任何 Rust 代码。允许空行和 `#` 注释,文件中的行序即列表展示顺序。

```jsonc
{
  "name": "claude-code", // 必填,唯一标识(CLI 中使用)
  "display": "Claude Code", // 必填,人类可读名称
  "global": {
    "env_home": {
      "var": "CLAUDE_CONFIG_DIR",
      "default": ".claude",
      "path": "skills",
    },
  },
  "detect": [
    { "env_home": { "var": "CLAUDE_CONFIG_DIR", "default": ".claude" } },
  ],
}
```

`global` 是单个路径规格;`detect` 是路径规格列表——只要其中任意一条解析到
已存在的路径,该 agent 即被视为已安装。每条规格只能包含以下键之一:

| 键                                                              | 解析为                                     |
| --------------------------------------------------------------- | ------------------------------------------ |
| `{"home": "..."}`                                               | `home/<path>`                              |
| `{"config": "..."}`                                             | `config/<path>`                            |
| `{"cwd": "..."}`                                                | `cwd/<path>`                               |
| `{"env_home": {"var": "...", "default": "...", "path": "..."}}` | `$VAR \|\| home/<default>`,再拼接 `<path>` |
| `{"env_var": {"var": "...", "path": "..."}}`                    | `$VAR/<path>`;变量未设置时不匹配           |
| `{"system": "/abs/path"}`                                       | 绝对路径;仅开启系统探测时才检查            |

agent 是否需要符号链接由 `agents.rs` 中的 `is_native` 判定:将解析后的 `global`
路径规格与 `~/.agents/skills` 比较——只有目录恰好等于它的 agent(如 cline、warp)
才是原生的,无需链接。厂商私有目录的 agent(如 Antigravity 的
`~/.gemini/config/skills`)需要建立真实的目录级符号链接。`universal` 伪 agent
的 `"detect": []` 使它永远不会被检测为已安装。

## 开发与测试

```bash
cargo build            # 构建
cargo test             # 运行全部测试
cargo clippy           # lint
cargo fmt              # 格式化
```

- **单元测试**内联在 `src/` 各模块（`#[cfg(test)]`），共享夹具见
  `src/core/test_utils.rs`。
- **集成测试**位于 `tests/`，通过 `assert_cmd` 驱动真实 CLI；`lib_api.rs`
  覆盖库 API。
- 示例：`cargo run --example manage`（临时目录上的生命周期演示）与
  `cargo run --example add_skill`（真实环境）。

## 发布

新版本一律通过 GitHub Actions 发布到 crates.io（见 `.github/workflows/`），
不要在本地手动 `cargo publish`。发布前：按语义化版本递增 `Cargo.toml` 的
`version`，在 `CHANGELOG.zh-CN.md`（及 `CHANGELOG.md`）记录变更，并同步各文档的
`*.zh-CN.md` 中文翻译。
