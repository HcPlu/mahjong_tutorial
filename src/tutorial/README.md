# `src/tutorial/` — AI 教程模块（复盘教练等）

本模块是 fork 的核心增量：把 `history`（对局事件）与 `analysis`（可解释数字）
重组为人能听懂的教学语言。LLM 只负责组织语言，**绝不产生数字、绝不决定打牌**
（D-0005）。

## 三区数据流（D-0007）

```
ReviewMaterial（讲解原料：analysis 引擎的数字 + 牌面快照）
      │
      ▼
┌─ 上传区 upload.rs ───────────────┐
│ outbound_allowed()  consent 门控  │  ← 出网唯一通道
│ redact()            剥离身份      │
│ build_messages()    渲染 prompt   │
└──────────────┬───────────────────┘
               ▼
   provider.rs::TutorialProvider   ← 唯一网络调用点
   （首发实现 OpenAiCompatProvider，
     OpenAI/DeepSeek/GLM/Kimi/本地 llama-server 同协议）
               │
               ▼
┌─ 下载区 download.rs ─────────────┐
│ parse_review_doc()               │  ← 入网唯一通道
│   · 提取 JSON（容忍首尾废话）      │
│   · 巡数/junme 一致性             │
│   · 反幻觉：数字与原料逐项相等     │
└──────────────┬───────────────────┘
               ▼
┌─ 保存区 store.rs ────────────────┐
│ TutorialStore                    │  ← 落盘唯一通道
│ <config_root>/tutorial/reviews/  │
│   <游戏ULID>.json   缓存优先      │
└──────────────────────────────────┘
```

## 不变量（改动前自查）

1. **单向流**：数据只沿 material → upload → provider → download → store
   流动。任何新代码不得跳区（例如在 provider 里拼 prompt、在 pipeline 里
   直接 reqwest、绕过 store 写文件）。
2. **consent 先于调用**：`TutorialConfig::outbound_allowed()`（enabled +
   upload_consent + llm 配置齐全）为 false 时，不得发起任何网络请求。
3. **缓存先于请求**：调 LLM 前必须先 `TutorialStore::load_review` 命中检查
   ——同一局复盘只花一次钱。
4. **数字的可信来源只有一个**：`EngineNumbers` 只能由 `analysis` 引擎填充；
   `download::parse_review_doc` 对 LLM 回显的数字做**逐项相等**校验，
   不相等整巡拒绝（降级为纯数字复盘），绝不"就地采信"。
5. **浮点回显用 round-trip 格式**：`upload` 渲染数字用 `{:?}`
   （round-trip 精确），这是 download 端能用 `==` 严格比对的前提。
   改渲染格式前先想清楚这一点。
6. **key 永不进日志/错误信息/仓库**：环境变量 `TUTORIAL_LLM_API_KEY`
   优先于 `config.toml`（见 `LlmConfig::effective_api_key`）。

## 关键类型

| 类型 | 位置 | 角色 |
|---|---|---|
| `ReviewMaterial` / `TurnMaterial` / `TurnContext` / `CandidateNumbers` / `EngineNumbers` / `ReviewCoverage` | `material.rs` | 输入侧：讲解原料（`TurnContext` 为仅输入侧的局况，不参与回显校验） |
| `ReviewDoc` / `TurnDoc` | `material.rs` | 输出侧：LLM 讲解 + 原样数字（与输入镜像；`overview` 为 D-0012 新增总评，旧缓存缺省可读） |
| `build_review_material` / `scan_decisions` / `DecisionPoint` | `assemble.rs` | M1 装配器：history 事件流 → tracker 重放 × analysis 重算 → 原料 |
| `ChatMessage` / `ChatRequest` / `ChatResponse` / `TutorialProvider` | `provider.rs` | LLM 抽象缝（D-0004） |
| `TutorialConfig` / `LlmConfig` | `src/config/tutorial.rs` | `[tutorial]` 配置节（含 consent） |

注意：`upload.rs` 的 system prompt 里手写着 JSON 输出契约（字段名与
`ReviewDoc` 的 serde 形状一致）。**改 `ReviewDoc` 字段时必须同步改那段
prompt 文本**——两处没有编译期联动，靠这条 README 约束。

## 装配器（`assemble.rs`，D-0012）

`scan_decisions(events, seat)` 用私有 `GameTracker` 逐事件重放，在每个
**我方摸牌→切牌窗口**产出 `DecisionPoint`。要点：

- **StartGame 座位注入**：tracker 的观察视角取自 `StartGame.id`；历史流
  录制时若未标视角（id 为 None），装配器克隆该事件注入 `id = seat` 再喂，
  否则快照手牌全是 `"?"`。
- **junme 口径**：我方第 N 次摸牌（含杠后岭上），与 live 深讲同口径；
  不用快照 `turn_count`。
- **候选集**：`hand14.maintain` 前 3 ∪ 实际切牌 ∪ `best_defence_discard`
  （后两者仅当存在于 maintain——否则没有可引用的引擎数字），cap 5。
  `maintain` 为空的巡（如恰好完整的 14 张形）整巡跳过。
- **`prob` 语义**：展示候选内 `mixed_waits_score` 的归一化份额——引擎
  衍生的排序提示，**不是** bot 动作概率（live 深讲才用 bot 的 prob）。
- **数字映射**（`engine_numbers`，live/review 共用）：
  `shanten ← 候选 result.shanten`；`win_rate ← avg_agari_rate/100`；
  `deal_in_rate ← mixed_risk[切牌]/100`（引擎内是百分数）；
  `avg_score ← riichi_point>0 ? round : (dama_point>0 ? round : None)`。
- **关键巡规则**（`is_key_turn`，纯引擎数字、表驱动测试）：①实际切牌
  不在 maintain 前 3 ②对手立直且我方 ≤1 向听 ③该局首个可切听巡
  ④攻防最优切牌不同且均在候选。`ReviewCoverage::Key` 取关键巡 cap 12，
  `All` 全部决策巡。

## 配置示例

```toml
[tutorial]
enabled = true          # 功能总开关
upload_consent = false  # 隐私同意：允许（脱敏后的）牌局数据出网上云

[tutorial.llm]
base_url = "https://api.deepseek.com/v1"  # 含版本段；本地 llama-server 用 http://127.0.0.1:8080/v1
api_key  = ""                            # 或环境变量 TUTORIAL_LLM_API_KEY
model    = "deepseek-chat"
request_timeout_ms = 60000               # 离线批量，clamp 到 [5s, 300s]
```

## 扩展方法

- **换/加 provider**：新写一个实现 `TutorialProvider` 的结构体即可，
  pipeline 只认 trait。OpenAI 兼容的厂商直接改 `base_url` 就行。
- **新的教程产物**（课程、学员进度等）：走同样的三区套路——输出类型放
  `material.rs`（或新 types 文件），解析+校验进 `download.rs`，落盘进
  `store.rs` 的新子目录。不要为新产物开"后门"。
- **接 IPC（M1 流水线）**：在 `src/ipc/commands.rs` 加命令，调用顺序固定为
  store 查缓存 → `outbound_allowed` 门控 → `redact` → `build_messages` →
  `provider.complete` → `parse_review_doc` → 盖 `created_at`/`model` 章 →
  `store.save_review`。失败任何一步都降级，不重试轰炸。
- **实时讲解深讲层（M2，D-0008）**：设计见
  [docs/design/live-coach.md](../../docs/design/live-coach.md)。后端落点
  `live.rs`（订阅总线装配单巡 `TurnMaterial`，live 语境下 `actual` 填 bot
  首选动作）；`upload.rs` 增单巡渲染、`download.rs` 增单巡校验、`store.rs`
  增 `save_turn_doc`/`load_turn_doc`（`live/` 子目录）。模板层在前端，不经本模块。
- **自适应练习系统（M3，D-0009）**：设计见
  [docs/design/drills.md](../../docs/design/drills.md)。后端落点
  `src/tutorial/drills/`（题目 schema、生成管线、学习者模型、判题），判题
  与答案键 100% 本地，LLM 深讲走本模块三区（缓存 `drill_explanations/`）。

## Don't

- 不要 `use crate::ipc`（方向只能是 ipc → tutorial）。
- 不要 import `crate::bot` 的内部函数；脱敏逻辑在 `upload.rs` 自持一份，
  与 MJOT 的 `build_api_events` 保持语义等价即可，不共享代码。
- 不要给 LLM 输出的数字"四舍五入后放行"之类宽容校验——宁可少一段讲解。
- 不要在 provider 层做缓存或重试；缓存属于 store，重试属于未来的 pipeline
  策略层（带成本意识）。
