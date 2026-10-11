# 设计文档 · 实时讲解（Live Coach，ROADMAP M2）

> 对应决策：[D-0008](../DECISIONS.md)（双层架构）。状态：设计定稿，未实施。
> 红线（继承 AGENTS.md §6）：数字只能来自 analysis 引擎；consent 先于调用、缓存先于请求；`src/tutorial/` 不 `use crate::ipc`；用户可见字符串 i18n 四语言。

## 0. 一句话

对局中把「AI 建议切什么」升级为「为什么」：**模板层**用确定性规则从现有事件里即时拼话（零网络零成本），**深讲层**在用户点击时走 `src/tutorial/` 三区管线调一次 LLM 并缓存。

## 1. 数据流

```
                       ┌────────────────────────── 模板层（默认，纯前端） ──────────────────────────┐
PostTrackerBus ──► tracker ──► AnalysisBus ──► ipc 事件 `analysis-result` ──┐
                    └────────► BotResponseBus ──► ipc 事件 `bot-response` ──┼──► 前端 commentary 引擎
                                （决策点信号）        `get_game_snapshot` ───┘      （规则评估 + 节流）
                                                                              │
                                                                              ▼
                                                                     CommentaryTile 磁贴渲染
                       └──────────────────────────────────────────────────────────────────────┘

                       ┌────────────────────────── 深讲层（按需，经后端） ─────────────────────────┐
前端点击「详细讲解」 ──► IPC 命令 tutorial_explain_live ──► src/tutorial/live.rs（最近决策点原料）
                                                        ──► store 缓存查询 ──命中──► 直接返回
                                                        ──miss──► upload（consent 门控 + 脱敏 + 单巡 prompt）
                                                              ──► provider（唯一网络点）
                                                              ──► download（数字逐项核对）
                                                              ──► store 落盘缓存 ──► 返回 TurnDoc
                       └──────────────────────────────────────────────────────────────────────────────┘
```

要点：

- **模板层零后端改动**：`analysis-result` / `bot-response` 事件已把所需数字（`AnalysisResult` 全量 + `meta.show` 候选）送到前端，快照经既有命令 `get_game_snapshot` 拉取。不新增 event_bus 通道、不新增 Tauri 事件。
- **决策点信号**：后端 `TrackedEvent.can_act` 不直接出网，前端以 `bot-response` 事件到达（bot 只在我方决策点响应）作为"该出话了"的触发，配合 `turn_count` 去重。
- **深讲原料来自后端自己的总线**（实施时按 D-0012 修正）：`src/tutorial/live.rs` 在 `src/lib.rs` 装配时订阅 `PostTrackerBus`（StartGame 重置 session key / 我方 Tsumo 计 junme）+ `BotResponseBus`（决策点信号），收到 bot 响应时**直接锁共享 tracker 快照、`to_player_info` + `analyze` 即时重算**（与 analysis runner 同一纯函数链，~1ms）。原设计配对 `AnalysisBus` × `BotResponseBus`，但两个并行订阅者没有先后保证（bot 响应可能先于对应 analysis 结果发布），重算彻底消除配对时序问题；有界内存不变：只保留当前决策点，新的覆盖旧的。数字由后端从引擎重算，而不是信任前端回传——D-0005 的纪律在数据源头就成立。

## 2. 触发与节流（前端状态机）

| 规则 | 定义 |
|---|---|
| 总开关 | `[tutorial.coach] enabled`（默认 **false**），且受 `[tutorial] enabled` 总开关约束。可在设置 → AI 教程（LLM）卡片的「对局中教练」开关切换，也可手改 config.toml |
| 触发 | 收到 `bot-response` 且该巡（`bakaze + kyoku + turn_count` 为键）尚未出话 |
| 每巡上限 | 至多 1 条（`max_per_turn`，v1 固定为 1） |
| 规则去重 | 同巡所有命中规则按优先级排序，取最高 1 条渲染（并列时取维度靠前的） |
| 静音 | 磁贴上一键静音，直到**本场对局结束**（快照 `is_done` 或下一局 `StartGame` 重置为配置默认） |
| 无 bot 时 | 只有 `analysis-result` 也能出话（效率/防守族不依赖 meta），置信族规则跳过 |

## 3. 模板规则库

规则是纯函数数据，全部放前端：

```
frontend/src/components/tutorial/commentary/
  engine.ts      # evaluate(analysis, botResp, snapshot, state): CommentaryLine[]  —— 纯函数
  rules.ts       # 规则目录：{ id, family, priority, when(…), render(…) -> {i18nKey, params} }
  throttle.ts    # 每巡去重 + 静音状态机
  __tests__/     # vitest：规则命中/不命中、节流、静音
```

**规则的参数只能来自事件字段本身**（`AnalysisResult`、`meta.show.items[].prob`、快照字段），模板层没有任何计算新数字的自由度——这是 D-0005 在模板层的体现。首批五族示例：

| 族 | 例规则 | 条件（示意） | 输出句式（i18n key 示意） |
|---|---|---|---|
| 效率 | R-EF1 进张对比 | `hand14` 态，首优 `waits_total` 与次优差 ≥ 4 | `coach.efficiency.ukeire`：「切 {tile} 进张 {n} 张（次优 {tile2} 只有 {m} 张），保持 {shanten} 向听」 |
| 效率 | R-EF2 听牌宣告 | `hand13` 态且 `shanten == 0` | `coach.efficiency.tenpai`：「听牌！待 {tiles}，共 {left} 张」 |
| 防守 | R-DF1 安全牌指引 | 任一对手 `is_riichi` 且我方 `hand14` | `coach.defence.safe`：「{who} 立直，{tile} 是现物；无现物时 {tile2} 铳率最低（{p}%）」（`best_defence_discard` / `mixed_risk`） |
| 立直 | R-RC1 立直 vs 默听 | `hand13` 态听牌且 `riichi_point`/`dama_point` 均非空 | `coach.riichi.compare`：「立直期望 {rp} vs 默听 {dp}——{建议侧}更优」 |
| 押受 | R-PF1 局势提示 | 终局巡目 + 点数差超阈值 + `avg_agari_rate`/铳率语境 | `coach.pushfold.situation`：「南{n}局 {rank} 位，{攻/守}优先」（只叙述事实数字，不下结论时用中性句式） |
| 置信 | R-BT1 明确推荐 | `meta.show` 首项 `prob ≥ 0.6` 且与次项差 ≥ 0.3 | `coach.bot.confident`：「AI 明确推荐 {action}（置信 {prob}%）」 |
| 置信 | R-BT2 两难提示 | 首两项 `prob` 差 < 0.15 | `coach.bot.torn`：「AI 在 {a}（{p1}%）与 {b}（{p2}%）之间接近——通常是效率与安全的权衡」 |

规则库按族扩展（做加法不改引擎）；句式全部走 `coach.*` i18n 命名空间，四语言（en/ja/zh-CN/zh-TW）同步补齐。

## 4. 深讲层（后端，唯一 LLM 触发点）

### 4.1 原料装配（`src/tutorial/live.rs`）

订阅两条总线，在决策点（收到 `BotResponse`）把两侧数据配对成 `TurnMaterial`（复用 `material.rs` 现有类型，不新增结构）：

- `tehai`/`draw`/`junme` ← 快照（tracker 侧装配时直接从快照取，不经前端）；
- `candidates` ← `meta.show.items`（label + prob）× `AnalysisResult.hand14.maintain` 同名候选的数字，映射为 `EngineNumbers`（`win_rate ← avg_agari_rate`、`deal_in_rate ← mixed_risk[切牌]`、`avg_score ←` 期望点；取不到的字段保持 `None`，download 端核对只约束非空项）；
- **live 语境下 `actual` 填 bot 首选动作**（被解释对象；M1 复盘语境才是学员的实际选择）。此语义差异写进 `material.rs` 字段注释；
- `context` 除局况（风局/分数/立直家/宝牌）外还带**各家牌河与副露**（公开信息，渲染成
  `各家牌河: 我方(座位0)[…] 座位1[…] …`，供 LLM 做现物/对手速度推理）；system prompt 带
  一段紧凑的**规则/术语锚定**（立直麻将基本形、记号、向听/现物等术语），不塞完整规则书
  ——数字已由引擎算好，锚定只为叙述一致性，避免每巡多付上千 token。

### 4.2 调用链（固定顺序，失败即降级、不重试）

```
tutorial_explain_live()
  1. live.rs 取当前决策 material（无决策点 → 明确错误返回）
  2. store.load_turn_doc(key) 命中 → 返回缓存           # 缓存先于请求
  3. outbound_allowed(cfg)? → 否则返回「未授权出网」错误   # consent 先于调用
  4. redact(material) → build_turn_messages(...)          # upload.rs 新增单巡渲染，
                                                          #   与 build_messages 共享逐巡渲染函数
  5. provider.complete(req)                               # 唯一网络点，带超时
  6. parse_turn_doc(content, material)                    # download.rs 新增单巡校验：
                                                          #   recommended ∈ 候选、numbers 逐项相等、
                                                          #   explanation_zh 非空（复用既有校验件）
  7. 盖 model / created_at 章 → store.save_turn_doc(key)
```

任何一步失败：返回结构化错误，前端磁贴保持模板层内容并 toast 提示——**没有深讲也不影响模板层**。

### 4.3 缓存键（已知妥协）

live 阶段对局还没有 history ULID（ULID 在对局结束落盘时才定）。v1 缓存键 = `session_key + "-t" + junme`，其中 `session_key` 为对局开始时的本地时间戳（`YYYYMMDD-HHMMSS`）+ 座位。落盘在 `<root>/live/` 下，id 过 `store.rs::validate_game_id`。M1 流水线落地时若能让 history 提前分配对局 id，则切换为 ULID 键；旧缓存直接作废（量小，无迁移价值）。`TutorialStore` 增 `save_turn_doc` / `load_turn_doc` 两个方法，写路径仍走单 mutex。

### 4.4 自动模式（D-0016，摸牌时抢先生成）

`[tutorial.coach] auto_policy = "all"` 时，`drive_loop` 在建好当巡素材后立刻把它投进
`mpsc(4)` 队列（满则丢新——只有 reconnect 突发会满，该丢的恰好是过期 restore 巡）。
独立的 `auto_explain_worker` 串行消费：

- **每个 job 实时重读 config**：`coach.enabled` / `auto_policy` / `outbound_allowed()`
  任何一项中途关闭，下一个 job 立即跳过（正在途的那一个请求无法撤回）；
- **缓存优先**：`load_turn_doc` 命中也发事件（前端错过原事件可追补），与手动按钮
  共用 `pipeline::generate_live_turn`（provider → trust 解析 → 落盘），同巡只花一次钱；
- 结果经 `TutorialLiveBus`（`event_bus.rs` 定义）广播 `TutorialLiveEvent`，ipc 转发为
  前端事件 `tutorial-live-turn`；失败只 log、不重试——下一巡是天然重试。运行日志（info 可见）：成功记 `live turn N explained in X.Xs (model=…, cache=miss)`、缓存命中记 `served from cache`、outbound-not-allowed 每进程 warn 一次（缺 key/未同意不再静默）。

**不做 streaming**：`TurnDoc` 是整块 JSON，解析需要完整响应，流式无法提前渲染。
实测单巡时延（glm-5.3-flashx）：reasoning_effort=high 约 4-17 秒，low 约 3-8 秒；
切牌窗口 10-20 秒，**抢先模式建议配 low**。

## 5. 配置

```toml
[tutorial]
enabled = true            # 总开关（已有）
upload_consent = false    # 出网同意（已有）

[tutorial.coach]          # 新增子表，config/tutorial.rs 追加 CoachConfig
enabled = false           # 实时教练默认关（ToS 与干扰顾虑，见 ROADMAP 非目标）
max_per_turn = 1          # 每巡提示上限（v1 固定 1）
deep_dive_button = true   # 是否显示「详细讲解」入口（真正出网仍需 upload_consent）
auto_policy = "off"       # D-0016：off = 仅手动按钮；all = 每次摸牌后台抢先生成一巡讲解
```

模板层**不涉及出网**，只受 `coach.enabled` 约束；深讲层额外要求 `outbound_allowed()`（即 `tutorial.enabled && upload_consent && llm.is_configured()`）。

## 6. 前端呈现

- **磁贴**：`frontend/src/tiles/CommentaryTile.tsx`（新文件，上游目录内的 fork 增量）。按磁贴五处登记惯例接线：`tiles/defaults.ts`（`TileId` / `ALL_TILES` / `DEFAULT_LAYOUTS` + `DEFAULT_LAYOUTS_3P` / `TILE_TITLES`）、`tiles/registry.tsx`（`renderTile` case）、四语言 `tile.commentary`。`defaults.ts` 与 `registry.tsx` 是上游文件，改动为纯追加并在 PR 描述申报（AGENTS 规则 1）。
- **磁贴内容**：当前讲解句（Mahgen 渲染句中牌名）+「详细讲解」按钮 + 静音按钮 + 深讲结果展开区（`TurnDoc.explanation_zh` + 数字表）。
- **自动模式呈现**（D-0016）：`useTauriBridge` 监听 `tutorial-live-turn` 写入 `coach.autoDoc`；摸牌（bot-response）时置 `autoPending`——磁贴先显示「AI 讲解生成中…」，事件到达自动渲染讲解框；静音时完全抑制；新鲜的 autoDoc 优先于可能过期的手动 deepDoc。
- **store**：`tutorialStore` 增加 coach 切片（开关/静音/当前句/深讲 doc + loading），事件接进 `hooks/useTauriBridge.ts` 既有分发点（不另开监听）。
- **overlay 窗口呈现**（D-0018 落地）：`routes/Overlay.tsx` 在建议列表下方渲染自动讲解面板（「第N巡」+ 标题 + 正文，`tileNamesZh` 转中文牌名，`line-clamp` 限高；bot-response 置 pending、`tutorial-live-turn` 落讲解）；header 增「讲解」按钮（紫色，与 AUTO 并列）——一键 `set_coach_auto`：开 = `coach.enabled + auto_policy="all"`，关 = 仅降 `auto_policy`。事件经 `app.emit` 全窗口广播、capability 已含 `core:event:default`，无需动 overlay 三件套；`drive_loop` 每决策现读 config，翻转下一次决策即生效。

## 7. 测试策略

- `commentary/engine`：vitest 纯函数测试——构造 fixture（典型 `AnalysisResult` + `meta.show`）断言规则命中与参数取值；静音/去重状态机测试。
- `live.rs`：单测内自建 broadcast channel 发假事件，断言装配出的 `TurnMaterial` 字段与数字映射。
- `upload::build_turn_messages` / `download::parse_turn_doc`：沿用三区既有测试模式（身份不进 prompt、篡改数字拒绝、空讲解拒绝）。
- `TutorialStore::save_turn_doc/load_turn_doc`：round-trip + 非法键拒绝。

## 8. 非目标

- ~~不自动逐巡调 LLM（D-0008）~~、~~不新增 event_bus 通道~~——两条已被 D-0016（用户明示选择「摸牌时抢先」）收窄；~~不做 overlay 呈现~~——已被 D-0018 落地（讲解面板 + 一键自动）；不碰 `src/autoplay/`。
