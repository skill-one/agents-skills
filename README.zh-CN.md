# agents-skills

[![crates.io](https://img.shields.io/crates/v/agents-skills.svg)](https://crates.io/crates/agents-skills)
[![docs.rs](https://img.shields.io/docsrs/agents-skills.svg)](https://docs.rs/agents-skills)
[![CI](https://github.com/skill-one/agents-skills/actions/workflows/ci.yml/badge.svg)](https://github.com/skill-one/agents-skills/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

简体中文 | [English](README.md)

一个极简的 AI Agent 技能安装与管理工具：所有技能集中存放在一个**规范目录**，
通过一次 `agent --link` 即可让 [Claude Code](https://claude.com/code)、Codex、Cursor
等 70+ 编程 Agent 全部可见——安装一次，处处生效。

```bash
cargo install agents-skills
```

同时提供可嵌入的 Rust 库，见 [docs/LIBRARY.zh-CN.md](docs/LIBRARY.zh-CN.md)。

## 快速开始

```bash
agents-skills agent --link                  # 链接所有已安装的 agent
agents-skills add anthropics/skills@pdf     # 安装一个技能
agents-skills list                          # 查看已安装技能
```

## 工作原理

每个技能只在规范目录 `~/.agents/skills/<name>` 保存一份（被禁用的技能位于
`~/.agents/disabled-skills/<name>`）。`agent --link` 让每个已安装 agent 的
技能目录以符号链接指向它，因此之后安装的技能所有 agent 立即可见，无需同步。

```bash
agents-skills add owner/repo@pdf            # 从 GitHub 安装
agents-skills add ./my-skill                # 安装本地技能目录
agents-skills add owner/repo@pdf --ref v1.2 # 指定分支、标签或 commit SHA
agents-skills list --json                   # 机器可读输出
agents-skills remove pdf                    # 移除技能
agents-skills disable pdf                   # 禁用（文件保留）/ enable 为其逆操作
agents-skills agent --link claude-code      # 链接指定 agent
agents-skills agent --unlink claude-code    # 解除链接（已并入内容保留）
agents-skills agent --status                # 查看链接状态与私有内容
```

关键行为：

- **来源** —— `add` 只安装一个显式命名的技能：要么是直接包含
  `SKILL.md` 的本地目录，要么是 GitHub 上的 `owner/repo@<技能>`（按技能
  目录名大小写不敏感匹配；根目录下的 `SKILL.md` 用仓库名选择）。不加
  `--ref` 时使用默认分支。远程安装是**单次请求**：从 `codeload.github.com`
  下载整个仓库的 tarball——完全不使用 GitHub REST API，因此不存在 API
  速率限制。仅支持公开仓库；Git LFS 文件会以指针占位文件的形式安装。
- **绝不覆盖** —— 同名技能已安装（无论启用或禁用）时报告 `skipped`；想替换
  请先 `remove`。
- **链接单向并入存量内容** —— 技能目录移入规范目录，其他文件移入其内的
  `.misc/<agent>/`，同名冲突保留已有副本。`--unlink` 只断开链接，不会把内容移回。

## 文档

- [docs/CLI.zh-CN.md](docs/CLI.zh-CN.md) —— 完整命令参考
- [docs/LIBRARY.zh-CN.md](docs/LIBRARY.zh-CN.md) —— 可嵌入的 Rust 库
- [docs/DEVELOPER.zh-CN.md](docs/DEVELOPER.zh-CN.md) —— 架构与贡献指南
- [CHANGELOG.zh-CN.md](CHANGELOG.zh-CN.md) —— 版本变更（English: [CHANGELOG.md](CHANGELOG.md)）

## License

在以下任一许可证下授权：

- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT license（[LICENSE-MIT](LICENSE-MIT)）

由你选择。
