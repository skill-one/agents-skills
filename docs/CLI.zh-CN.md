# agents-skills 命令行参考

简体中文 | [English](CLI.md)

`agents-skills` CLI 的完整命令参考。功能概览见 [README](../README.zh-CN.md)，库使用者见 [LIBRARY.zh-CN.md](LIBRARY.zh-CN.md)。

## 安装

```bash
cargo install agents-skills
```

全局选项：`-v, --version` 显示版本号。

## 命令速查表

| 命令        | 说明                  |
| --------- | ------------------- |
| `add`     | 从来源安装技能包            |
| `remove`  | 移除已安装技能             |
| `list`    | 列出已安装技能             |
| `disable` | 禁用已安装技能             |
| `enable`  | 重新启用已禁用的技能          |
| `agent`   | 链接/解除链接/查看 agent 状态 |

命令不设别名（极简接口，只认全名）。

通用说明：技能存放在规范目录（全局 `~/.agents/skills` 或项目 `.agents/skills`）。
默认操作**全局**作用域；`-p/--project <目录>` 切换到**项目**作用域，目录值必填且
必须已存在，操作该目录下的 `.agents/skills`（当前目录写 `--project .`）。

## add

从本地路径、Git 仓库或 HTTPS 端点安装技能包。

```
agents-skills add <source...> [options]
```

| 选项                    | 说明                  |
| --------------------- | ------------------- |
| `-p, --project <dir>` | 安装到指定项目目录（默认全局）     |
| `-s, --skill <s>...`  | 要安装的技能名（`'*'` 表示全部） |
| `-l, --list`          | 仅列出可用技能，不安装         |

```bash
agents-skills add anthropics/skills               # 安装到全局 ~/.agents/skills
agents-skills add anthropics/skills --project .   # 安装到当前项目 .agents/skills
agents-skills add anthropics/skills@pdf           # 仅安装指定技能
agents-skills add anthropics/skills -l            # 只列出可用技能
```

安装后运行 `agents-skills agent --link` 让 agent 可见（`add` 不自动链接）。

## remove

从规范目录移除已安装技能。

```
agents-skills remove [skills...] [options]
```

| 选项                    | 说明                 |
| --------------------- | ------------------ |
| `-p, --project <dir>` | 从指定项目目录移除（默认全局）    |
| `-s, --skill <s>...`  | 要移除的技能（`'*'` 表示全部） |
| `--all`               | 移除全部技能（含已禁用的）      |

```bash
agents-skills remove pdf      # 移除指定技能
agents-skills remove --all    # 移除全部技能
```

## list

列出已安装技能，附每个技能的描述与安装时间。各 agent 的链接状态用
`agent --status` 查询。

```
agents-skills list [options]
```

| 选项                    | 说明                  |
| --------------------- | ------------------- |
| `-p, --project <dir>` | 列出指定项目目录的技能（默认全局）   |
| `--json`              | JSON 输出（机器可读）       |

```bash
agents-skills list
agents-skills list --json
agents-skills list --project .
```

每个技能打印两行 —— 名称与描述，随后是 `路径 [状态] · <本地安装时间>`：

```
Project Skills

docx Create and edit Word documents, including tables and headers.
  .agents/skills/docx [enabled] · 2026-09-19 12:54
```

`list --json` 每个技能输出相同字段：

| 字段            | 说明                                        |
| ------------- | ----------------------------------------- |
| `name`        | 技能名（取自 `SKILL.md` frontmatter）            |
| `description` | 技能描述，已规整为单行                               |
| `path`        | 技能当前所在目录（规范目录，或 `disabled-skills`）        |
| `enabled`     | `true` 在规范目录，`false` 停放在 `disabled-skills` |
| `installedAt` | 技能目录的创建时间，Unix 秒（UTC）；无法获取时为 `null`       |

`installedAt` 是"技能落到磁盘的时间"的近似值：`add` 安装是精确的，但从 agent
目录并入的技能会保留该目录原本的创建时间，且对同名技能重新安装会刷新它；在
不记录创建时间的文件系统（部分 Linux 文件系统）上为 `null`。

## disable / enable

`disable` 把技能目录移到 `disabled-skills/`，使其对所有 agent 不可见；`enable`
移回规范目录恢复可见（`disable` 的逆操作）。文件完整保留，无损可逆。

```
agents-skills disable [skills...] [options]
agents-skills enable  [skills...] [options]
```

| 选项                    | 说明                 |
| --------------------- | ------------------ |
| `-p, --project <dir>` | 项目作用域，指定项目目录（默认全局） |
| `-s, --skill <s>...`  | 目标技能（`'*'` 表示全部）   |
| `--all`               | 禁用所有已启用 / 启用所有已禁用  |

```bash
agents-skills disable pdf      # 禁用指定技能
agents-skills disable --all    # 禁用全部已启用技能
agents-skills enable  pdf      # 启用指定技能
agents-skills enable  --all    # 启用全部已禁用技能
```

## agent

管理 agent 技能目录与规范目录的链接关系。

```
agents-skills agent [agents...] (--link | --unlink | --status) [options]
```

| 选项                    | 说明                                |
| --------------------- | --------------------------------- |
| `-p, --project <dir>` | 操作指定项目目录的技能目录（默认全局）               |
| `--link`              | 把 agent 技能目录链接到规范目录（存量内容自动并入）      |
| `--unlink`            | 解除 agent 与规范目录的链接（已并入内容留在规范目录）     |
| `--status`            | 查看链接状态（只读）                        |

`--link`/`--unlink`/`--status` 互斥，须指定其一。`--status` 区分两种可见状态：
原生读取规范目录的 agent（Codex、Cursor、Warp 等）标记 `(canonical dir)`，符号
链接接入的标记 `(linked)`。对**未链接**的 agent，会分类列出其自身技能目录中的
内容：`private skills: ...` 为技能（子目录及指向目录的符号链接），`other files: ...`
为其他文件，即链接时会被并入的内容。
agent 默认为自动探测结果，`'*'` 表示全部。

链接对已有内容的处理（并入是单向的，不可撤销）：

- 目录为空：直接替换为链接。

- 其他情况：先并入内容再建立链接 —— 技能目录移入规范目录，非技能条目移入规范
  目录内的 `.misc/<agent>/`（点目录，不会被误判为已安装技能），随后 agent 目录
  被链接替换。项目级还会写入 `.misc/.gitignore`，让被隔离的文件不进版本控制。

- 同名冲突一律丢弃 agent 侧副本、保留已有副本：规范目录副本优先，已禁用
  （`disabled-skills`）的技能保持禁用、不被重新导入；指向规范目录的旧模型
  单技能符号链接同样丢弃（其内容已在规范目录）。

- 已并入的内容**不会**在 `--unlink` 时归还；此后由 `remove`/`disable` 管理。

- 仅一种情况拒绝：目录本身是指向别处的符号链接。

```bash
agents-skills agent --link                       # 链接全部已安装 agent
agents-skills agent --link claude-code           # 链接（存量内容自动并入）
agents-skills agent --status                     # 查看链接状态
agents-skills agent --unlink claude-code         # 解除指定 agent 链接
```

## 相关概念

- 来源格式与安装位置见 [README · 功能说明](../README.zh-CN.md#功能说明)。

- 库接口（每个命令对应的 `Manager` 方法与请求/结果类型）见 [LIBRARY.zh-CN.md](LIBRARY.zh-CN.md)。

