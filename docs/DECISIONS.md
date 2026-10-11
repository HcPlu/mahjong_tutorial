# 决策日志（Decision Log）

轻量 ADR：一条决策一节，编号递增（D-000N），只记**影响架构、目录、依赖或产品方向**的决定。
新条目追加在最上方。格式：背景 → 决定 → 理由/代价。

---

## D-0017 · 对局时顺带记录 bot 应答（botlive），复盘区分「对局 bot」与「复盘 bot」

- **背景**：仓库主人 2026-10-11 指出：复盘页的 bot 参照是复盘时用**当前配置的** bot 重放的，与**对局当时实际在打**的 bot 可能不同（换 bot、换权重、换温度），混为一谈会误导教学；要求对局过程中直接把 bot 的每次应答（动作 + meta.show 候选分布）记录下来，复盘时两种参照并存且明确区分。
- **决定**：
  1. **记录点 = `tutorial::live::drive_loop` 的决策窗口**（bot 应答落在 `build_live_turn` 成功处）：此刻 session_key / junme / 分类三要素齐备，用 `bot::replay::classify_reaction`（bot 公开 API，AGENTS §6.4 允许）把应答转成 `BotRefTurn` 追加进会话缓冲；`StartGame` 开新会话（记录 bot 名：按 `num_players` 读当时配置的 active bot），`EndGame`/下一个 `StartGame` 落盘 `botlive/<session_key>.json`（`LiveBotDoc`，含 seat 与 RFC3339 `started_at`）。纯本地数据，与 botref 同类，无 LLM、无 consent 门。
  2. **复盘匹配**：`tutorial_get_live_bot(game_id)` 读 history 记录的 `(our_seat, started_at..ended_at)`，在 botlive 目录里找 seat 相等且 `started_at` 落入 `[record.started_at−60s, record.ended_at+60s]` 的所有会话（断线重连/页面重载会把一局拆成多个 live 会话），按 junme 合并、后开始的会话覆盖同 junme——重连的 restore 重放会把 junme 快进回真实值，覆盖即对齐。产出复用 `BotRefDoc` 形状（`bot` = 最后会话的 bot 名），前端零新类型。
  3. **复盘 UI 双源**：候选图表在复盘 bot（紫色，现状）之外并排渲染对局 bot（青色）的置信度与「对局实选/首选」徽标；两个来源都有时概率标签加「对局/复盘」前缀，只有单一来源时保持现状文案。表头标注两边的 bot 名。
- **理由**：对局应答本来就在 `BotResponseBus` 上流过，不记录是纯浪费；「当时怎么打的」和「现在会怎么打」对照正是复盘教学的核心问题之一；复用 `BotRefTurn`/`classify_reaction` 让两套数据可直接对比。
- **代价**：每局多一个 ~10-40KB 的本地 JSON（无清理，量级可忽略）；live 会话与 history 记录靠时间窗匹配而非强外键（两边都在同一进程内对同一个 `StartGame` 盖时间戳，误差毫秒级，60s 容差绰绰有余）；极端时钟漂移下匹配失败 = 复盘退回无对局参照，不损数据。

## D-0016 · 对局中自动讲解：摸牌时抢先生成 + 串行 worker + TutorialLiveBus 推送

- **背景**：M2 深讲层现状是纯手动（点「详细讲解」才调 LLM，等待 3-8 秒）。仓库主人 2026-10-11 要求对局中自动 AI 分析，并在三个触发方案中明确选择**「摸牌时抢先」**——摸牌瞬间（bot 响应到达、切牌窗口打开）即后台生成，讲解赶在切牌决策前出现。这收窄了 live-coach.md §8「不自动逐巡调 LLM（D-0008）」与「不新增 event_bus 通道」两条旧非目标。
- **决定**：
  1. **触发点 = `drive_loop` 收到 `BotResponse`**（此刻正是决策窗口打开）：`coach.enabled && coach.auto_policy == "all"` 时把当巡 `TurnMaterial` 投进 `mpsc(4)` 队列（满则丢新——只有 reconnect 突发会满，恰好该丢的是过期 restore 巡）。`actual` 语义不变（bot 首选；玩家此刻还没切）。
  2. **串行 worker**（`auto_explain_worker`）：单消费者、同一时刻最多一个在途 LLM 请求；**每个 job 实时重读 config**（consent/开关中途关闭立即生效）；缓存优先（命中也发事件，前端错过事件可追补）；生成走与手动按钮共用的 `pipeline::generate_live_turn`（provider→trust 解析→落盘），同巡只花一次钱。失败只 log 不重试。
  3. **新总线 `TutorialLiveBus`**（定义仍在 `event_bus.rs` 唯一合法处）承载 `TutorialLiveEvent{session_key, junme, doc, model}`，ipc 转发为前端事件 `tutorial-live-turn`；前端磁贴先显示「生成中」，事件到达自动渲染，静音时完全抑制。**不做 streaming**：TurnDoc 是整块 JSON，流式无法提前渲染。
  4. **`auto_policy: off(默认)|all`** 进 `CoachConfig`；`coach.enabled` 是主开关（磁贴关了就不许偷偷花钱）。缓存键仍 `{session_key}-t{junme}`，与手动按钮共享。
- **理由**：抢先模式下时延即价值（实测 glm-5.3-flashx + reasoning_effort=high 单巡 4-17 秒，配 low 可到 3-8 秒，切牌窗口 10-20 秒）；串行避免并行限流；实时读配置保证隐私开关的即时性；缓存共享让自动生成的巡点手动按钮零成本。
- **代价**：全部巡触发则每局 60-80 次调用（flashx 约 ¥0.2-0.3/局，免费模型可零成本）；`reasoning_effort=high` 时讲解经常晚于切牌（建议配 low）；为抢时延讲解里没有玩家实际切牌的对比（弃牌后对比式讲解可作后续增强）。



## D-0015 · Progress 日志模块：docs/progress/ 按日记录 prompt 与产出

- **背景**：本仓库为 AI 驱动开发（vibe），过程资产（主人给过的 prompt、每次改动的缘由）此前只散落在 git 提交信息与被 ignore 的 `.zcode/plans/` 里，会话结束即丢失；ROADMAP 管里程碑、DECISIONS 管架构决策，日常「做了什么、为什么做」无人承载。仓库主人 2026-10-11 明示需要 progress 监控。
- **决定**：新增 `docs/progress/` 模块——按日一文件（`YYYY-MM-DD.md`，追加式）记录每条 prompt（原文照抄、超 15 行截断+摘要；分类对齐 Conventional Commits 另加 `ask`/`decision`）与动作/产出/提交；`INDEX.md` 倒序总览；`scripts/progress-report.sh` 出统计。记录义务写入 AGENTS §5，约束所有 agent session。
- **理由**：prompt 原文是 vibe 项目最不可再生的资产（复盘「当时为什么这么改」全靠它）；markdown 追加式零依赖、git 天然版本化；与 ROADMAP/DECISIONS 分层不重叠。
- **代价**：每轮收工多一步记录（秒级）；日志与 git 历史部分冗余；prompt 原文入库意味着过程对话随之公开（个人项目可接受，密钥按规则永不记录）。

## D-0014 · 删除 AGENTS.md 硬性规则第 8 条；新增「最小实现」等工作规则

- **背景**：仓库主人 2026-10-11 明示两件事：其一，第 8 条（教程定位/代打红线）经 D-0011 收窄后无需再作为硬性规则存在；其二，现行工作方式导致代理产出防御性、过度复杂的代码和过重的测试，要求转向最小实现。
- **决定**：
  1. 硬性规则第 8 条整条删除，后续条目重排编号。代打治理不再是 Agent 硬性规则；产品层面的表述仍见 ROADMAP 非目标与 D-0011（如需进一步调整另议）。
  2. 新增三条硬性规则：**按最小需求实现**（不防御性编程、不过度设计，测试只做最简验证）；**每次修改完成后立即 commit**；**经常核对 AGENTS §3「架构速览」与代码现状**。
- **理由**：个人教程项目优先迭代速度与代码简洁；治理红线由决策日志与 ROADMAP 承载即可。
- **代价**：autoplay 约束的强制性下降（依赖决策记录自觉遵守）；「实时指导默认不干扰对局」不再由硬性规则保证（D-0008 的教练开关默认关仍是事实实现）。

## D-0013 · 现阶段讲解输出走「信任直通」；严格反幻觉校验暂停但保留

- **背景**：真实对局验证（6 局 × GLM-5.3-flashx）暴露严格逐项数字校验的工程成本：17 位浮点回显差 1 ULP、模型把 6 位小数自行舍成 3 位、偶发推荐候选之外的牌。虽已通过「源头量化 + 抢救模式」把 6/6 局跑通，但每次模型行为漂移都要再调校验参数，拖慢现阶段的核心目标（把教程闭环跑起来）。
- **决定**（仓库主人 2026-10-11 明示）：**现阶段直接保留 LLM 输出**——`parse_review_doc_trust` / `parse_turn_doc_trust` 只做 JSON 提取与身份盖章（game_id/seat/junme 由本地断言），不逐项核对数字、不丢弃巡。D-0005 的严格校验器保留在 `download.rs`（连同单测），前端渲染的引擎数字仍来自本地 `analysis`（候选表、回放页），受影响的只是讲解文本内嵌数字的可信度。
- **理由**：现阶段优先验证产品闭环与讲解质量；数字精度问题已被源头量化（和率 2 位/铳率 3 位小数）大幅缓解；校验器随时可切回，不存在不可逆损失。
- **代价**：讲解文本中的数字理论上可能失真（引擎候选表不受影响）；面向外部用户发布前应恢复严格校验或加上「AI 生成，数字未核对」的显著标注。

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
