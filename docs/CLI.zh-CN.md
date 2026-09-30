# agents-skills CLI 参考

简体中文 | [English](CLI.md)

`agents-skills` CLI 的完整命令参考。功能概览见 [README](../README.zh-CN.md)；
库使用者见 [LIBRARY.zh-CN.md](LIBRARY.zh-CN.md)。

全局选项：`-v, --version`。所有命令都只操作规范目录 `~/.agents/skills`；被禁用
的技能停放在 `~/.agents/disabled-skills`。命令不设别名，只认全名。

## add

从本地目录或 GitHub 安装恰好一个技能。

```
agents-skills add <source> [--ref <ref>]
```

`<source>` 要么是本地技能目录，要么是 GitHub id `owner/repo/slug`，指定 GitHub
上的一个技能。两种情况下，**技能**都是指其 `SKILL.md` frontmatter 声明了非空
`name` 的目录（`description` 可选）——其他情况作为本地来源会被拒绝，远程发现也
不会命中。

id 的最后一段是技能的 **slug**：由 frontmatter `name` slug 化而来（小写、每个
空格替换为 `-`、`/` 删除、其余字符原样保留——如 `agent development` →
`agent-development`）。会检查仓库 tree 中的每个 `SKILL.md`，第一个 slug 化
`name` 与请求 slug 相等的 manifest 生效（最浅优先、路径优先）。仓库根目录的
`SKILL.md` 也是普通候选：命中时安装整个仓库。安装的目录保留被命中目录在源仓库
中的原始目录名——而技能的身份（`list` 报告、`remove`/`enable`/`disable` 选择的
依据）是 slug；frontmatter 中的原始 `name` 仅作展示（`list --json` 中的
`displayName`）。

| 选项          | 说明                                          |
| ------------- | --------------------------------------------- |
| `--ref <ref>` | 指定分支、标签或完整 commit SHA（仅 GitHub 来源） |

```bash
agents-skills add ./my-skill                        # 安装本地技能
agents-skills add anthropics/skills/pdf             # 从 GitHub 安装一个技能
agents-skills add anthropics/skills/pdf --ref v1.2  # 指定分支、标签或完整 commit SHA
```

远程安装是单次请求：从 `codeload.github.com` 下载整个仓库的 tarball，解包到
临时目录后在本地选择匹配的技能目录。完全不使用 GitHub REST API，因此不存在
API 速率限制；仅支持公开仓库，Git LFS 文件以指针占位文件形式安装。不加
`--ref` 时使用默认分支。其他来源形式（裸 `owner/repo`、完整
URL、子路径、GitLab/SSH、HTTPS 压缩包）均会被拒绝，报错会指明受支持的语法。

`add` 绝不覆盖：同名技能已安装（无论启用或禁用）时报告 `skipped`；想替换请先
`remove`。安装后运行 `agent --link` 让 agent 可见。

## remove

从规范目录移除技能（同时移除 `disabled-skills` 中停放的副本）。技能按 slug
选择——即 `list` 报告的名字——大小写不敏感匹配。

```
agents-skills remove [skills...] [--all]
```

| 选项                 | 说明                       |
| -------------------- | -------------------------- |
| `-s, --skill <s>...` | 要移除的技能名             |
| `--all`              | 移除全部技能（含已禁用的） |

```bash
agents-skills remove pdf      # 移除指定技能
agents-skills remove --all    # 移除全部技能
```

## list

列出已安装技能，含描述、路径、状态与安装时间。

```
agents-skills list [--json]
```

每个技能输出两行——名称与描述，然后是
`路径 [状态] · <本地安装时间>`：

```
Skills

docx Create and edit Word documents, including tables and headers.
  ~/.agents/skills/docx [enabled] · 2026-09-19 12:54
```

`--json` 每个技能输出一个对象：

| 字段          | 说明                                                               |
| ------------- | ------------------------------------------------------------------ |
| `name`        | 技能的 slug——frontmatter `name` slug 化后的形式，所有命令选择的身份。manifest 缺失 `name` 的目录不是技能，不会被列出 |
| `displayName` | frontmatter 中声明的原始 `name`——仅作展示（可能含空格与大小写）  |
| `description` | 技能描述，已规整为单行                                             |
| `path`        | 技能的真实磁盘目录（启用时在规范目录，禁用时在 `disabled-skills`）  |
| `enabled`     | 在规范目录中为 `true`，停放在 `disabled-skills` 中为 `false`       |
| `installedAt` | 技能目录创建时间（Unix 秒，UTC），不可用时为 `null`                |

各 agent 的链接状态用 `agent --status` 查看。

## disable / enable

`disable` 把技能目录移入 `disabled-skills/`，对所有 agent 隐藏；`enable` 移回。
文件完整保留——无损、可逆。技能按 slug 选择——即 `list` 报告的名字——大小写
不敏感匹配。

```
agents-skills disable [skills...] [--all]
agents-skills enable  [skills...] [--all]
```

| 选项                 | 说明                            |
| -------------------- | ------------------------------- |
| `-s, --skill <s>...` | 要禁用 / 启用的技能名           |
| `--all`              | 禁用全部已启用 / 启用全部已禁用 |

幂等：重复执行会报告技能已处于该状态；不存在的名称报告为 missing 而非报错。

## agent

管理各 agent 技能目录与规范目录之间的链接。

```
agents-skills agent [agents...] (--link | --unlink | --status)
```

| 选项       | 说明                                                |
| ---------- | --------------------------------------------------- |
| `--link`   | 将 agent 技能目录链接到规范目录（存量内容会被并入） |
| `--unlink` | 解除与规范目录的链接（已并入内容保留在规范目录）    |
| `--status` | 查看链接状态（只读）                                |

三种模式互斥。agent 默认为自动检测到的集合；`'*'` 表示全部。`--status` 把原生
读取规范目录的 agent 标为 `(canonical dir)`，通过符号链接接入的标为
`(linked)`；对未链接的 agent，会分类列出其自身目录内容——`private skills`
与 `other files`——即链接时会被并入的内容。

`--link` 对存量内容的处理（并入是单向的）：

- 空目录直接替换为链接。
- 否则先并入内容：技能目录移入规范目录，其他条目移入其内的
  `.misc/<agent>/`（点目录，不会被误认为技能），然后 agent 目录变为链接。
- 同名冲突保留已有副本（规范目录优先；已禁用的名称保持禁用）。指向规范目录
  的旧式逐技能符号链接直接丢弃——其内容本就在规范目录中。
- `--unlink` 不会恢复已并入的内容；之后用 `remove`/`disable` 管理。
- 仅一种情况拒绝：该目录本身是指向别处的符号链接。

```bash
agents-skills agent --link                       # 链接所有已安装 agent
agents-skills agent --link claude-code           # 链接（存量内容自动并入）
agents-skills agent --status                     # 查看链接状态
agents-skills agent --unlink claude-code         # 解除指定 agent 的链接
```
