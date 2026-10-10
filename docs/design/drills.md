# 设计文档 · 自适应练习系统（Drills，ROADMAP M3）

> 对应决策：[D-0009](../DECISIONS.md)。状态：设计定稿，未实施。
> 红线（继承 AGENTS.md §6）：判题与讲解数字只能来自 analysis 引擎；出网必经 upload.rs、入网必经 download.rs、落盘必经 store.rs；consent 先于调用、缓存先于请求；`src/tutorial/` 不 `use crate::ipc`；i18n 四语言。

## 0. 一句话

一套**决策点题库 + 学习者模型**的训练系统：引擎按目标难度出题（生成后自验并产出答案键），模型按学员水平自适应选题（甜蜜区难度 + 弱点定向），作答即时本地判题（margin 分级 + 数字对比反馈），LLM 只在用户要讲解时把引擎数字组织成教学语言。

## 1. 参考与差异

| | Mahjong Efficiency Trainer（euophrys） | nagchan 何切練習系列 | 本系统 |
|---|---|---|---|
| 题目 | 随机 14 张纯效率题 | 人工精选，场况完整（巡目/点数/座位/dora） | 程序化生成 + 精选种子 + 历史挖掘，场况完整 |
| 题型 | 切牌（效率） | 切牌/立直-默听/副露/防守 | v1 切牌（效率+防守），v2 立直押受/副露 |
| 难度 | 无体系 | 人工标注（难/中高） | 引擎参数化（margin/向听/场况复杂度），连续可调 |
| 选题 | 无 | 线性系列 | **自适应**：学习者模型 + 甜蜜区 + 弱维度加权 + 错题重排 |
| 判题 | 进张数对比 | 无（看作者解答） | 引擎答案键 margin 分级（最优/可接受/失误） |
| 讲解 | 一行进张差 | 作者长文说理 | 数字反馈模板句 + bot 候选参照 + **按需 LLM 深讲**（数字逐项核对） |

取 Trainer 的交互骨架（点选切牌、即时反馈、"连练到手听"session 模式），取 nagchan 的场况呈现与题型多样性模板；护城河在最右列后四行。

## 2. 训练循环（系统主图）

```
            ┌────────────────────────────---── 一个题步（step）────────--------------┐
            │                                                                    │
 学习者模型 ──► 选题策略 ──► 题目(DrillPrompt) ──► 学员作答 ──► 判题(引擎答案键)      │
    ▲          (甜蜜区+弱点)     (不含答案键)        点选切牌      margin 分级        │
    │                                                          │                  │
    │            ┌── 更新模型(EWMA/升降级/错题入队) ◄────────────┘                  │
    └────────────┤                                                               │
                   反馈面板: 分级结果 + 你的选择vs最优数字对比 + (可选)LLM 深讲      │
            └────────────────────────────────------────────------------------------┘
```

刻意练习三原则的落点：**甜蜜区难度**（选题策略命中预期正确率 7–8 成）、**即时反馈**（作答后同步出分级 + 数字对比，LLM 讲解异步可选）、**弱点定向**（弱维度加权 + 错题 spaced requeue）。

## 3. 题目 schema（`src/tutorial/drills/types.rs`）

```rust
enum Dimension { Efficiency, Defence, RiichiPushFold, Calls, Endgame }   // 技能维度
enum QuestionType { Dahai, RiichiOrDama, CallOrPass }                    // v1 只实现 Dahai
enum DrillSource { Procedural, Curated, Mined }

struct DifficultyParams {        // 同时是「出题约束」与「难度特征」——同一参数集两用
    level: u8,                   // 课程表级别 L1..L5
    margin_class: MarginClass,   // Decisive | Moderate | Close（最优-次优差距档位）
    shanten: i8,                 // 目标向听（效率维度）
    junme_range: (u8, u8),
    opponent_riichi: bool,       // 防守维度主约束
    score_pressure: bool,        // 终局/点数情境
}

struct DrillScenario {           // 字段集与 analysis::PlayerInfo34 输入对齐，缺省即无该信息
    num_players: u8,
    bakaze: String, kyoku: u8, honba: u8, oya: u8, seat: u8, junme: u8,
    scores: Vec<i32>,
    dora_indicators: Vec<String>,
    tehai: Vec<String>,          // 14 张（含摸牌，最后一张为 draw）
    melds: Vec<MeldLike>, own_discards: Vec<String>,
    opponents: Vec<OpponentLike>,// 河（含 tedashi/立直标记）+ 副露 + 立直状态
}

struct AnswerKeyEntry {          // 引擎排名，首名 = 最优
    label: String,               // mjai 风格，如 "dahai:5s"
    numbers: EngineNumbers,      // 复用 material.rs（shanten/win_rate/deal_in_rate/avg_score）
    metric: f64,                 // 维度主指标（效率=进张数；防守=铳率%），判题 margin 用
}

struct Drill {
    id: String,                  // 过 validate_game_id（防路径穿越，沿用 store.rs 惯例）
    dimension: Dimension,
    question_type: QuestionType,
    difficulty: DifficultyParams,
    scenario: DrillScenario,
    answer_key: Vec<AnswerKeyEntry>,        // 服务端持有，不下发前端
    draw_schedule: Vec<String>,             // 仅「连练到手听」模式：预采样的后续摸牌序列
    source: DrillSource,
    explanation_hints: Option<HintLike>,    // curated 题的人工说理（只进反馈，不进判题）
    seed: Option<u64>,                      // Procedural 题可复现
}
```

全部结构 serde `#[serde(default)]` + 手写 `Default`（向后兼容旧 JSON，见 AGENTS 陷阱清单）。**下发给前端的 `DrillPrompt` 剥掉 `answer_key` 与 `metric`**（判题在后端，答案不提前出门——单机应用防的不是作弊而是自欺）。

## 4. 难度参数化与生成管线（`src/tutorial/drills/generate.rs`）

```
(维度, 级别) ──► 课程表查出 DifficultyParams
   ──► 约束采样器（带 seed 的 RNG）：随机手牌/场况，满足硬约束（向听数、巡目区间、对手立直…）
   ──► 构造 PlayerInfo34 ──► analysis::analyze()
   ──► 验证器（算实际难度特征，不满足则弃题重采样，上限 ~200 次）：
        · 低级别要求 margin_class ∈ Decisive（首优显著领先）且干扰项少
        · 高级别要求 ∈ Close（存在真实权衡：效率 vs 安全 / 打点 vs 速度）
        · 局面合理性（手牌可构成、河与副露自洽）
   ──► answer_key = 引擎候选排名（见下），连同难度特征落盘
```

**答案键的维度化主指标**（v1 两维度）：

| 维度 | 排名依据 | metric | 可接受档 ε |
|---|---|---|---|
| 效率 | `hand14.maintain` 按 (向听, `waits_total`, `avg_next_shanten_waits`) 排序 | 进张张数 | 与最优差 ≤ 2 张且向听相同 |
| 防守 | 候选按 `mixed_risk[tile]`（铳率%）排序，现物优先 | 铳率% | 与最优差 ≤ 1.0 个百分点 |

立直与押受（`riichi_point` vs `dama_point` + 点数语境）、副露在 v2 扩展（题型 `RiichiOrDama` / `CallOrPass`）。

生成成本与缓存：一次 `analyze()` 量级 ms–百 ms（集成测试 bench 需 `--release` 才有意义——生成器同样只在 release 下做批量预生成）。按 `(维度, 级别)` 惰性生成并缓存（首访触发，每级封顶如 200 题），题目 id 用 `参数哈希 + seed`，同 seed 完全可复现。

**精选种子（curated）**：题包 JSON 仿 nagchan 范式（场况完整 + 难度标注 + 人工说理 `explanation_hints`）。**入库时引擎复算并覆盖答案键**——人工答案只作为出题意图参考，判题永远以引擎为准（数字纪律优先于权威感）。规则/役速查等纯教学内容并入题包的非题目页面（课程内容与代码分离）。

## 5. 学习者模型与选题策略（`src/tutorial/drills/learner.rs`）

```rust
struct DimensionProgress {
    ewma: f64,              // 掌握度：最优=1.0 / 可接受=0.5 / 失误=0，EWMA α=0.3
    attempts: u32,
    level: u8,              // 当前练习级别
}
struct Progress {
    per_dimension: BTreeMap<Dimension, DimensionProgress>,
    requeue: Vec<RequeueEntry>,     // { drill_id, due_at_count }  错题间隔 3/7/15 题重现直至答对
    recent: VecDeque<String>,       // 最近 N 题id（选题时排除）
    // 会话统计：连对、正确率、各维度答题数
}
```

**选题策略**（`tutorial_drill_next` 内执行，纯函数可测）：

1. `requeue` 到期题最优先（最早到期者）；
2. 否则选维度：加权随机，权重 ∝ `(1 - ewma)` × 久未练习加成（弱点定向）；
3. 选级别：升降级规则（v1 启发式——连续 2 组 ≥80% 升级、<50% 降级），使预期正确率落在 **0.7–0.8** 甜蜜区；Elo 式 item–learner 双参数 rating 是演进方向，不进 v1；
4. 从该 `(维度, 级别)` 题池排除 `recent` 后随机取一。

**冷启动**：某维度 `attempts < 5` 时按课程表从 L1 线性推进（ROADMAP 风险表已列此对策）。进度以后端 `progress.json` 为权威（每次判题后更新落盘），前端 `tutorialStore` 只持镜像。

## 6. 判题与反馈（`src/tutorial/drills/grade.rs`）

```
tutorial_drill_submit(drill_id, step, answer)
  → answer == key[0]                              → Optimal  （最优）
  → |metric(answer) − metric(key[0])| ≤ ε_dim      → Acceptable（可接受，接近最优）
  → 其他                                           → Mistake  （失误，展示最优）
```

反馈面板（纯本地、即时）：

- **分级徽章**（最优/可接受/失误）+ 模板句（`drill.feedback.*` i18n，四语言）；
- **数字对比表**：你的选择 vs 最优（维度主指标 + 向听/和率/铳率等 `EngineNumbers`），数字全部来自答案键；
- curated 题附 `explanation_hints` 人工说理；
- **「AI 讲解」按钮**（可选异步）：把该题该步装配成 `TurnMaterial`（`actual := 学员的选择`，`candidates := 答案键前几名`），走与 M1/M2 完全相同的三区管线（upload 单巡渲染 → provider → download 数字逐项核对 → 缓存 `drill_explanations/<drill_id>.json`）。consent 门控统一适用：即使题目是合成局面，**任何 LLM 调用都要求 `upload_consent`**（单一心智模型；M4 的 mined 题含真实对局数据，更必须如此）。

## 7. 存储布局

```
<config_root>/tutorial/                      # 由 util::resolve_dir 解析，与 history/ 平级
  reviews/<ulid>.json                        # M1 复盘缓存（已有）
  live/<session_key>-t<junme>.json           # M2 实时深讲缓存（见 live-coach.md）
  drills/
    curriculum.json                          # 课程表：维度×级别定义（难度参数、解锁/过关规则）——内容与代码分离
    packs/<pack_id>/pack.json                # 精选题包（元数据 + 题目 + 人工说理 + 教学页）
    generated/<dim>-L<n>/<hash>.json         # 程序化生成题缓存
  drill_explanations/<drill_id>.json         # 练习 LLM 深讲缓存
  progress.json                              # 学习者模型 + 进度（后端权威）
```

所有文件读写必经 `store.rs`（新增 drills 相关 save/load 方法，写路径单 mutex），id 一律过 `validate_game_id`；格式演化用 serde `#[serde(default)]` 向后兼容，不写迁移脚本。

## 8. IPC 命令面（ipc → tutorial 单向，命令定义在 `src/ipc/commands.rs` 追加区）

| 命令 | 入 | 出 | 说明 |
|---|---|---|---|
| `tutorial_drill_next` | 偏好提示（维度偏好/模式，可选） | `DrillPrompt`（无答案键） | 选题策略执行 + 缺题时触发生成 |
| `tutorial_drill_submit` | `drill_id, step, answer` | `DrillGrade`（分级 + 对比数字 + 连练模式下一手） | 判题 + 更新模型落盘 |
| `tutorial_drill_explain` | `drill_id, step, answer` | `TurnDoc` | 三区深讲，缓存优先 |
| `tutorial_progress_get` | — | `Progress` | 前端镜像刷新 |

**连练到手听（ToTenpai）session 模式**：效率维度专用。题内预采样 `draw_schedule`（满足墙余牌约束），每步判题后 `submit` 返回下一摸牌与新 `tehai`，向听归零即结束；后端无会话状态（`step` 计数由前端回传，`draw_schedule` 随题下发——泄露未来摸牌只是自欺，可接受，注释说明）。

## 9. 前端

```
frontend/src/routes/Tutorial/
  TutorialHome.tsx        # 维度掌握度总览（MasteryPanel）+ 课程表 + 「开始练习」
  TutorialExercise.tsx    # 练习播放器（单题 / 连练到手听），session 内连对/正确率
frontend/src/components/tutorial/
  drill/ContextBar.tsx        # 场况条：巡目/场风/座位/点数/dora（nagchan 式呈现）
  drill/HandChoices.tsx       # Mahgen 14 张手牌，点选即作答（Trainer 式即时反馈）
  drill/AnswerFeedback.tsx    # 分级徽章 + 数字对比表 + 人工说理 + 「AI 讲解」按钮
  drill/MasteryPanel.tsx      # 维度×级别掌握度（recharts，参考 RiskChart 磁贴用法）
  commentary/                 # M2 实时讲解规则引擎（共享目录，见 live-coach.md）
frontend/src/stores/tutorialStore.ts   # 当前题/session + progress 镜像 + M2 coach 切片
```

新路由登记三处：`main.tsx` 路由表、`lib/menuList.ts` 侧边栏、四语言 `nav.tutorial` 与 `drill.*` / `tutorial.*` 键。牌面渲染复用 `Mahgen.tsx` + `lib/tileIdx.ts`（`mjaiToMahgen`）。

## 10. 测试策略

- **生成器属性测试**（proptest 式或固定 seed 集）：局面可解析（`PlayerInfo34` 构造成功）、答案键与引擎一致（对存量题重跑 `analyze()` 结果不变）、难度特征落在目标区间、同 seed 完全复现、重采样上限不挂死。
- **判题单测**：三档分级边界（恰在 ε 上/下）、未知 label 拒绝、连练模式 step 推进与结束条件。
- **学习者模型单测**：EWMA 更新、升降级阈值、requeue 3/7/15 间隔与答对出队、冷启动线性路径、选题策略的弱点加权与排除 `recent`。
- **schema round-trip**：`Drill`/`Progress`/`curriculum.json` 旧版本 JSON（缺字段）能被新结构 `#[serde(default)]` 读入。
- **curriculum 校验**：级别难度参数单调性（高级别的 margin_class 不允许全 Decisive 等）。

## 11. 分期

- **v1（= ROADMAP M3 验收范围）**：`Dahai` 题型 × 效率（L1–L3）+ 防守（L1–L2）；单题 + 连练模式；EWMA 模型 + 启发式升降级；curated 首包（规则速查 + 若干种子题）。
- **v2**：`RiichiOrDama` / `CallOrPass` 题型、效率/防守高级别、Elo rating、mined 来源（M4 接入）。
