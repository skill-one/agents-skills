# agents-skills

[![crates.io](https://img.shields.io/crates/v/agents-skills.svg)](https://crates.io/crates/agents-skills)
[![docs.rs](https://img.shields.io/docsrs/agents-skills.svg)](https://docs.rs/agents-skills)
[![CI](https://github.com/skill-one/agents-skills/actions/workflows/ci.yml/badge.svg)](https://github.com/skill-one/agents-skills/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

简体中文 | [English](README.md)

一个极简的 AI Agent 技能安装与管理工具：所有技能集中存放在一个**规范目录**，
通过一次 `agent --link` 即可让 [Claude Code](https://claude.com/code)、Codex、Cursor 等
70+ 编程 Agent 全部可见可用——安装一次，处处生效。

```bash
cargo install agents-skills
```

同时提供可嵌入的 Rust 库，见 [docs/LIBRARY.zh-CN.md](docs/LIBRARY.zh-CN.md)。

## 快速开始

```bash
agents-skills agent --link            # 链接所有已安装的 agent
agents-skills add anthropics/skills   # 安装技能包（装进规范目录后所有 agent 立即可见）
agents-skills list                    # 查看已安装技能
agents-skills agent --status          # 查看各 agent 的链接状态
```

核心是链接：技能只在规范目录保存一份（项目级 `.agents/skills/` 或全局级
`~/.agents/skills/`），`agent --link` 为每个已安装的 agent 在其技能目录创建指向
规范目录的符号链接；之后 `add` 安装的技能所有 agent 立即可见，无需同步。

## 功能说明

`add`/`remove`/`update`/`disable`/`enable` 只操作规范目录，agent 通过符号链接自动
共享。常用命令：

```bash
agents-skills agent --link claude-code            # 链接（存量内容自动备份）
agents-skills agent --link claude-code --migrate  # 链接并把存量技能迁入规范目录
agents-skills agent --unlink claude-code          # 解除链接（并恢复备份内容）
agents-skills add anthropics/skills@pdf            # 仅安装指定技能
agents-skills list --json                          # 机器可读输出
agents-skills remove pdf                           # 移除指定技能
agents-skills update                               # 按 lockfile 更新到最新版本
agents-skills disable pdf                          # 禁用（移出规范目录，文件保留）
agents-skills enable pdf                           # 重新启用（disable 的逆操作）
```

- 默认操作全局作用域 `~/.agents/skills`;`--project <目录>` 切换到项目级（在指定
  目录的 `.agents/skills` 下，当前目录写 `--project .`）。
- `agent --link` 遇到 agent 技能目录已有内容时不拒绝：整个目录原样移入备份槽
  `.agents/backup-skills/<agent>/skills/`，`agent --unlink` 时整体恢复；加 `--migrate`
  则把其中的技能移入规范目录（同名时以规范目录为准，agent 侧副本留在备份）。
- `agent --status` 对未链接的 agent，分类列出其自身技能目录中的内容
  （`private skills` / `other files`）以及待恢复的备份（`backup parked at`）；
  已链接/规范目录的 agent 内容由 `list` 展示。
- 已禁用的技能 `update` 会跳过；`list` 始终展示全部技能并标注 `enabled`/`disabled`。

### 来源格式

`add` 的 `<source>` 参数支持：

| 格式                  | 示例                                                                   |
| ------------------- | -------------------------------------------------------------------- |
| 本地路径                | `./my-skill`, `/abs/path/skill`                                      |
| GitHub 简写           | `owner/repo`                                                         |
| GitHub / GitLab URL | `https://github.com/owner/repo`, `https://gitlab.com/group/repo`     |
| SSH / git URL       | `git@github.com:owner/repo.git`                                      |
| HTTPS 下载            | `https://example.com/skills.zip`, `.../skills.tar.gz`, 原始 `SKILL.md` |

GitHub 来源的常用写法(GitLab 相同,只是把 `/tree/...` 换成 `/-/tree/...`):

| 想安装的内容          | 写法                                                    |
| --------------- | ----------------------------------------------------- |
| 仓库中的全部技能        | `owner/repo`                                          |
| 只装一个技能          | `owner/repo@pdf`(等价于 `--skill pdf`)                   |
| 只装某个子目录         | `owner/repo/skills/pdf`                               |
| 指定分支、标签或 commit | `https://github.com/owner/repo/tree/v1.2`             |
| 指定 ref 下的某个子目录  | `https://github.com/owner/repo/tree/v1.2/skills/pdf`  |
| 子目录中的一个技能       | `agents-skills add owner/repo/skills/pdf --skill pdf` |

Git 来源的规则:

- ref(分支 / 标签 / commit SHA)只能写在 URL 里:GitHub 用
  `/tree/<ref>[/<path>]`,GitLab(含自建实例)用 `/-/tree/<ref>[/<path>]`。
  简写、纯仓库 URL 以及 SSH / git URL 没有 ref 语法,始终解析到默认分支。
- 简写不能同时携带子目录和 `@skill`,请改用 `--skill`。不带 `/tree/` 的 GitHub /
  GitLab URL(如 `https://github.com/owner/repo/skills/pdf`)和 `/blob/`
  文件 URL 会被显式报错拒绝,而不是静默装错范围。

HTTPS 来源直接下载并解压(zip / tar / tar.gz 压缩包,或单个文件如原始 `SKILL.md`)。

仓库内按优先级容器目录发现技能(`skills/`、`.curated/`、`.experimental/`、
`.system/`),浅层遮蔽深层。

### 安装位置

- **规范目录（唯一真实副本）** —— 项目级 `./.agents/skills/<name>`，全局级
  `~/.agents/skills/<name>`。
- **Agent 集成** —— 不原生读取规范目录的 agent 获得目录级符号链接：
  `.claude/skills` → `../.agents/skills`（项目）或 `~/.claude/skills` →
  `~/.agents/skills`（全局）。

## 命令速查表

| 命令        | 说明                    |
| --------- | --------------------- |
| `add`     | 从来源安装技能包              |
| `remove`  | 移除已安装技能               |
| `list`    | 列出已安装技能               |
| `update`  | 将技能更新到最新版本            |
| `disable` | 禁用已安装技能               |
| `enable`  | 重新启用已禁用的技能            |
| `agent`   | 链接/解除链接/查看 agent 链接状态 |

> 完整命令行参考见 [docs/CLI.zh-CN.md](docs/CLI.zh-CN.md)；库使用者见
> [docs/LIBRARY.zh-CN.md](docs/LIBRARY.zh-CN.md)；项目开发者见
> [docs/DEVELOPER.zh-CN.md](docs/DEVELOPER.zh-CN.md)。

## License

在以下任一许可证下授权：

- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT license（[LICENSE-MIT](LICENSE-MIT)）

由你选择。
