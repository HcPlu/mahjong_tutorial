# 决策日志（Decision Log）

轻量 ADR：一条决策一节，编号递增（D-000N），只记**影响架构、目录、依赖或产品方向**的决定。
新条目追加在最上方。格式：背景 → 决定 → 理由/代价。

---

## D-0012 · M1 复盘装配落地：analysis-only 候选、关键巡两档、live 深讲以重算替代总线配对；secret/ 永不入库

- **背景**：三区骨架（D-0007）就绪后，M1 复盘流水线（history → 原料 → LLM → 前端）与 M2 深讲层进入实施。两个待定问题：候选从哪来（AGENTS §6 禁止依赖 `src/bot` 内部，bot 离线重放无现成封装）；覆盖范围（一局 50-70 个自有决策点，全讲则每巡浅、输出长）。
- **决定**：
  1. **装配器 `src/tutorial/assemble.rs`**：私有 `GameTracker` 逐事件重放 + 决策点（我方摸→切窗口）跑 `analysis`。候选 = `hand14.maintain` 前 3 ∪ 实际切牌 ∪ `best_defence_discard`（后两者仅当在 maintain 中——否则无引擎数字可引用）；`maintain` 为空的巡跳过。**v1 不含鸣牌/杠决策**，bot/mortal 复算与分歧点挖掘留给 M4。
  2. **`prob` 语义分语境**：复盘侧 = 展示候选内 `mixed_waits_score` 的归一化份额（引擎衍生排序提示，非 bot 概率）；live 深讲侧 = `meta.show` 的真 bot 概率。
  3. **数字映射**（live/review 共用一函数）：`shanten ← result.shanten`、`win_rate ← avg_agari_rate/100`、`deal_in_rate ← mixed_risk/100`、`avg_score ← riichi_point>0 ? round : (dama_point>0 ? round : None)`。引擎内为百分数。
  4. **覆盖两档**（用户选择）：`key`（默认）= 确定性关键巡规则 cap 12——①实际不在 maintain 前 3 ②对手立直且 ≤1 向听 ③该局首听巡 ④攻防最优不同且均在候选；`all` = 全部决策巡，LLM 调用按 ≤15 巡/批分批。
  5. **junme 口径统一**：我方第 N 次摸牌（含岭上），不用快照 `turn_count`。
  6. **StartGame 座位注入**：tracker 观察视角取自 `StartGame.id`；历史流未标视角时装配器注入 `our_seat` 再喂，否则快照手牌全 `"?"`。
  7. **live 深讲以重算替代配对**（修正 D-0008 设计）：原设计订阅 `AnalysisBus`+`BotResponseBus` 配对，但两个并行订阅者无先后保证；实施改为收到 bot 响应即锁共享 tracker 快照重算（与 runner 同一纯函数链，~1ms），数字在源头自算。
  8. **context/overview 扩展**：`TurnMaterial.context`（风局/本场/分数/立直家/宝牌，仅输入侧不校验）与 `ReviewDoc.overview`（总评）随本决策加入，serde 默认向后兼容旧缓存；`secret/`（本地 LLM key）加入 `.gitignore`，key 只经 env `TUTORIAL_LLM_API_KEY` 或 Settings UI 使用。
  9. **厂商预设**（2026-10 价格调研）：免费主力 GLM-4.7-Flash（`open.bigmodel.cn/api/paas/v4`），付费甜点 qwen-flash（dashscope compatible-mode），质量位 DeepSeek-Flash / Kimi K2.6；预设只填 base_url/model，key 永远手输。
- **理由**：候选不依赖 bot 内部即可满足讲解需求（引擎排名 + 实际/防守补充已足够教学）；关键巡两档在成本（免费档也省 token）与讲解深度间取得平衡；重算消除竞态且代码更少；座位注入是历史流可重放的必要修正。
- **代价**：复盘候选无 bot 概率语义（用引擎份额近似），与 mortal 的分歧分析推迟到 M4；`all` 档长局需 4-5 次串行 LLM 调用（免费档无成本压力，仅时间）。



## D-0011 · 对局内代打开关：复用上游 autoplay 引擎，治理红线从"不增强"收窄为"不开发新能力"

- **背景**：仓库主人需要对局中随时切换 auto/手动。上游 Akagi 的 autoplay 引擎（CDP 点击 + 延迟模型 + 点击校验）完整可用，但其唯一入口是 Settings 的草稿+保存流程，对局中切换要离开牌桌。原治理规则（AGENTS.md 规则 8、ROADMAP 非目标）写的是"不新增、不增强代打能力"，一个对局内开关严格来说构成"增强"。
- **决定**：治理红线收窄——保持"**不开发新的代打能力/引擎**"（不做 MITM 帧注入式代打、不写新的点击自动化、不碰 `src/autoplay/` 引擎本体），但允许为上游既有引擎提供**便利 UI**：新增 `set_autoplay_enabled` IPC（即时生效：共享配置 + 持久化 + false→true 时拉起 once-per-process manager）与 overlay 悬浮窗内的 AUTO 开关（乐观更新、失败回退、随 `overlay-config` 广播与 Settings 保存双向同步）。
- **理由**：开关本身不产生任何新的代打能力，只是把既有能力的入口搬进对局；红线收窄由仓库主人明示，教育定位与"实时指导可开关"原则不变。
- **代价**：autoplay 的可用性从此是本 fork 的（个人使用）承诺的一部分；对局内一键代打降低了使用门槛，排位滥用与封号风险由使用者自担（模型卡亦警告勿用于排位）。

## D-0010 · mortal-298k 本地 bot：包装入库，权重与机器配置留在 ignore

- **背景**：教程需要高强度参照 bot（候选讲解与复盘"标准答案"的来源之一）。公开可下载的最强权重是 HuggingFace `VoidShine/mortal-298k`（Mortal v4 半庄模型，MAKA 平均 S+），但它是 130 MB 的 AGPL 二进制，且引擎路径因机器而异。
- **决定**：`mjai_bot/mortal/` 的**包装代码入库**（bot.py 协议代理、manifest、单元测试、对局模拟器、README、HF 来源文件），根 `.gitignore` 的上游规则 `mjai_bot/mortal*` 收窄为三条例外（权重 `mortal_298k.pth`、自动生成的 `engine.toml`、测试输出 `game_log.jsonl`；`settings.toml` 已有通用忽略规则）。引擎本体（Mortal 克隆、venv、libriichi.pyd）永远在仓库外，AGPL 边界维持子进程隔离（bot.py ↔ mortal.py 仅 JSONL stdio）。
- **理由**：可复现——README 记录完整构建/校验流程并以 sha256 锁定权重版本；权重不进 git（体积、AGPL、上游本就不分发权重）；机器路径不进 git（换机即失效且泄露本地布局）。
- **代价**：换机器需按 bot README 重做一次性构建；权重升级需人工下载并同步 sha256。

## D-0009 · 练习系统 = 引擎参数化题库 + 学习者模型自适应选题，判题以本地引擎为准

- **背景**：教学模块需要定型。市面已有两类形态：Mahjong Efficiency Trainer（itch.io）——交互好但纯牌效率，无难度体系、无自适应、无解释；nagchan 何切練習系列——场况完整、有作者说理，但人工精选、不可交互、不因人施教。本 fork 的差异化优势：`analysis` 引擎既可出题又可阅卷、`BotResponse` 候选可作参照、LLM 可把数字讲成人话。
- **决定**：练习系统 = **分级决策题库 + 自适应选题**，三个核心机制：
  1. **题目难度由引擎参数化**：难度特征全部本地可算（最优与次优的 EV/进张 margin、势均力敌候选数、向听深度、巡目、对手是否立直、点数压力、安全牌张数等）。生成器按目标难度约束采样，引擎验证后连同答案键（引擎候选排名 + `EngineNumbers`）入库。同一参数集同时驱动「出题约束」与「自适应选题」。
  2. **学习者模型驱动自适应选题**：按技能维度（牌效率 / 防守 / 立直与押受 / 副露 / 终局点数）维护掌握度（v1 用滚动正确率 EWMA，预留 Elo 式 rating 演进），选题命中「预期正确率约七到八成」的甜蜜区，弱维度加权，错题间隔重排（spaced requeue）。题目目的是快速提升水平——甜蜜区难度 + 即时反馈 + 弱点定向，刻意练习三原则逐条落地。
  3. **判题与讲解双层**：判题与数字反馈 100% 本地（按答案键 margin 分级：最优 / 可接受 / 失误），无网无 key 完整可用；LLM 讲解按需触发，复用三区管线并逐项核对数字（D-0005 纪律）。
  题目来源三轨：程序化生成、精选种子题包（引擎复算其答案以保数字纪律）、用户历史挖掘（M4 接入）。完整人机对局模拟器列入非目标（见 ROADMAP）。
- **理由**：引擎既当出题人又当阅卷人，判题可验证、零边际成本；自适应以最低成本逼近「因人施教」；判题与讲解的可信度根基与 D-0005 一脉相承。
- **代价**：无沉浸式对局体验，连续决策类技能只能靠多题串联近似；自适应策略初期冷启动需按课程表线性推进。

## D-0008 · 实时讲解双层架构：本地模板为底、按需 LLM 深讲

- **背景**：实时讲解（对局中解释「AI 为什么这么建议」）若每个决策点自动调 LLM，一局几十次调用，成本、秒级延迟与平台 ToS 风险都不可接受为默认行为。
- **决定**：讲解分两层——
  1. **模板层（默认）**：前端确定性规则引擎订阅现有 `analysis-result` / `bot-response` 事件，即时拼装四语言讲解。零网络、零成本、毫秒级；随教练开关（默认关）启停。讲解里的数字直接取自事件字段，不经 LLM 之手。
  2. **深讲层（按需）**：仅用户显式点击「详细讲解」才发起，走 `src/tutorial/` 三区管线（`TurnMaterial` 单巡变体：upload 脱敏门控 → provider → download 数字核对 → store），按 `game_id + 巡目` 缓存，同一决策点只花一次钱。**绝不自动逐巡上云。**
- **理由**：实时性（模板毫秒级）与成本可控；每次上云都是用户主动行为，consent 语义干净；模板数字直绑事件字段，天然满足 D-0005。
- **代价**：模板层讲解质量取决于规则库覆盖度（规则不命中时退化为纯数字展示）；深讲层有秒级延迟，只在用户愿意等时使用。

## D-0007 · 教程 LLM 走云端 OpenAI 兼容 API；数据流分「保存/上传/下载」三区

- **背景**：D-0004 只定了 provider 抽象，未定首发形态。曾评估本地部署（llama.cpp + Qwen3.5，Arc A770 16GB 可跑 9B 全显存），但下载带宽、驱动链路和维护成本使其不适合作为首发路径。
- **决定**：
  1. `TutorialProvider` 的首个实现是 **OpenAI 兼容云端客户端**（`src/tutorial/provider.rs::OpenAiCompatProvider`，`POST {base_url}/chat/completions`）。OpenAI / DeepSeek / GLM / Kimi 等主流厂商均兼容该协议；本地部署（如 llama.cpp 的 llama-server）也是同一协议，回退本地只需改 `base_url`，trait 不变——D-0004 的可替换性由此保住。
  2. 教程模块的数据架构定为**三区单向流**：上传区（`upload.rs`，出网唯一通道：脱敏 + prompt 渲染 + consent 门控）→ provider → 下载区（`download.rs`，入网唯一通道：JSON 提取 + 结构校验 + 数字反幻觉核对）→ 保存区（`store.rs`，落盘唯一通道：`tutorial/reviews/<ulid>.json`，缓存优先）。三区互相只通过类型交互，`src/tutorial/` 不依赖 `src/ipc`，也不耦合 `src/bot` 内部。
- **理由**：云到端零运维、中文质量天花板更高；三区切分让"什么数据离开本机"有唯一审计点，隐私开关（`upload_consent`，默认关）落到实处。
- **代价**：讲解生成依赖网络与 key；无网时按 D-0006 验收标准降级为纯数字复盘。

## D-0006 · 教程功能以「复盘教练」为第一里程碑

- **背景**：ROADMAP 有四个方向（复盘、课程、实时教练、个性化），需要切入点。
- **决定**：M1 做复盘教练（见 docs/ROADMAP.md）。
- **理由**：全部原料已就绪（`src/history/` 有整局 mjai 流、`src/analysis/` 有数字依据、Review 前端骨架已存在）；离线场景无实时性与 ToS 顾虑；是"LLM 解释引擎输出"这一核心模式的最小验证。
- **代价**：零基础学员要等到 M2 才有内容可学。

## D-0005 · LLM 决不直接决定打牌，只负责解释

- **背景**：教程引入 LLM 后，存在"让 LLM 参与决策"的诱惑。
- **决定**：打牌建议一律来自可验证的分析引擎（`src/analysis/`）与 bot（`BotResponse`）；LLM 只把结构化数据组织成教学语言。
- **理由**：LLM 会幻觉，麻将决策需要可验证性；这也是产品可信度的根基。讲解中的每个数字必须来自本地引擎。

## D-0004 · LLM 接入走 provider 抽象，key 不进仓库

- **背景**：教程需要 LLM（复盘讲解、课程生成），但云端/本地、不同厂商的取舍因人和时期而异。
- **决定**：后端 `src/tutorial/` 内定义 `TutorialProvider` trait，本地与云端实现可替换；key 从 `AppConfig`（toml_edit 路径）或环境变量读取；所有调用带超时与降级。
- **理由**：成本与隐私可控（用户可换本地模型）；避免供应商锁定。

## D-0003 · 教程代码的目录落点

- **决定**：后端 `src/tutorial/`（建模块时配 README，遵守既有惯例）；前端 `frontend/src/routes/Tutorial/`、`frontend/src/stores/tutorialStore.ts`、`frontend/src/components/tutorial/`；治理文档 `docs/`。
- **理由**：与上游完全隔离，合并 upstream 时零冲突；同时遵守仓库"每模块一个设计文档"的惯例。

## D-0002 · 治理文档用中文，代码与提交用英文

- **决定**：`AGENTS.md`、`CONTRIBUTING.md`、`docs/*` 用中文；代码、注释、提交信息、i18n key 沿用上游英文惯例。
- **理由**：治理文档的主要读者是本 fork 的维护者与 AI 代理（均可无碍读中文）；代码层保持英文可继续与上游互通、降低合并摩擦。

## D-0001 · 以「fork + 目录隔离」方式跟踪上游 Akagi

- **背景**：本仓库 fork 自 shinkuan/Akagi `v3`，上游活跃（每日 liqi 自动更新、持续发版）。
- **决定**：主分支保持 `v3` 与上游同名同历史；fork 改动全部放新目录/新文件；不改写上游代码，必要修改最小化并在 PR 中申报；定期 `git merge upstream/v3`。
- **理由**：上游的协议适配（liqi.proto、tenhou 变更）和缺陷修复是教程项目的免费引擎，值得持续吸收。
- **代价**：偶发冲突需要人工处理，但被目录隔离压缩到最小。
