# Progress 日志模块（docs/progress/）

> 治理依据：[D-0015](../DECISIONS.md)。本模块填补 ROADMAP（里程碑）与 DECISIONS（架构决策）之间的空隙：
> 按日记录**做了什么**、**仓库主人给过哪些 prompt**（AI vibe 项目的过程资产）、每次改动的简要留痕。

## 文件组织

- `YYYY-MM-DD.md`：按日一文件，当日内按时间正序**追加**；只追加，不重写历史条目。
- `INDEX.md`：倒序总览，每行一日；「主要成果」列是 agent 手写的语义总结。数字以脚本现算为准。
- 统计：`bash scripts/progress-report.sh`（见下），不落盘。

## 条目格式

每条 prompt 一个小节。标题格式固定，供 `progress-report.sh` 解析，**勿改 `·` 分隔符与字段顺序**：

````markdown
## P3 · 16:10 · feat · 一句话标题（这条请求是什么）

- **Prompt**：用户 prompt 原文照抄；超过 15 行截断，写「（已截断，共 N 行）」并补一句摘要。
- **动作**：做了什么，2–5 行。
- **产出**：改动的主要文件 / 模块。
- **提交**：相关 commit 短哈希（可多条）；条目自身所在的提交无法预知哈希，先写提交标题，下次记日志时补记。
- **状态**：done / wip。
````

- 编号 `P1、P2…` 当日内递增；时间取本机时间（`date +%H:%M`）。
- 回填条目（事后凭 git 记录补写）在 **Prompt** 字段写「（回填，原文未留存）」+ 依据提交信息的一句概括，时间取提交时间戳；可按主题把多条提交归并为一条。
- `ask` / `decision` 类即使不改代码也要记录；`decision` 若涉架构/目录/依赖/产品方向，另按 AGENTS §5 落 DECISIONS.md。

## Prompt 分类

| 标签 | 含义 |
|---|---|
| `feat` / `fix` / `refactor` / `docs` / `test` / `chore` | 与 Conventional Commits 同名类型对应的工作请求（混合提交组取主导类型） |
| `ask` | 咨询、讨论、看代码——不一定产生改动 |
| `decision` | 仓库主人的方向性指令 |

## 记录规则（Agent 行为契约，写入 AGENTS §5）

1. 任何提出新需求、要求改动、提问或决策的 prompt = 一条；同一轮连续琐碎追问（「继续」「好」等）合并进上一条。
2. **收工前**完成当日文件追加并同步 INDEX.md 一行；有新提交的补哈希。
3. 密钥、token 永不进日志；prompt 按原语言照抄（通常为中文）。
4. 不重写、不删除既有条目；写错了追加一条勘误。
5. 多 agent 并行写同一天文件时直接文末追加即可。

## 统计脚本

```bash
bash scripts/progress-report.sh             # 近 14 日明细 + 全部日志分类总计
bash scripts/progress-report.sh --days 30   # 指定天数
```

数据源：`docs/progress/[0-9]*.md` 的条目标题 + `git rev-list` 按日计数，纯 bash/awk、零依赖。
