# 参考文档 · Mortal 引擎内幕（模型架构与动作输出）

> 性质：**外部依赖的源码阅读笔记**，非本仓库设计。供 M4（学员 vs mortal 分歧挖掘，见
> [D-0012](../DECISIONS.md)）、live 深讲的 bot 概率语义（D-0008/D-0012）、以及任何要解释
> "AI 为什么这么打"的教程功能参考。
> 信息源：本机 Mortal 引擎检出（`mjai_bot/mortal/settings.toml` 的 `mortal_dir` 所指目录，
> upstream [Equim-chan/Mortal](https://github.com/Equim-chan/Mortal)，commit `0cff2b5`，2025-09-28）
> 与权重 `mortal_298k.pth`（v4，`conv_channels=192`、`num_blocks=40`，实测 Brain 10.8M + DQN 头
> 48K ≈ **10.9M 参数**，名字里的 298k 是 RL 步数）。核对日期 2026-10-11。
> 注意：上游演进后本文可能过期；换检出/换权重后应按 §7 源码索引重新核对。

## 0. 一句话

Mortal = **libriichi 把牌局编码成 (1012×34) 特征张量 → 一维 ResNet（Brain）提特征 → 决斗
DQN 头输出 46 维 Q 值 → 非法动作掩码置 −inf → argmax → 动作 id 解码成 mjai 事件**。
分类器式输出，不是生成式；**决策本体 100% 神经网络**，只有两条 rule-based 护栏旁路（§5）。

```
mjai 事件流 ──► PlayerState（规则引擎镜像牌局）
                 │ encode_obs(v4)
                 ▼
        obs (1012×34)  +  mask (46 合法动作位向量)
                 │ rayon 并行编码
                 ▼
        Brain: Conv1d(1012→192) → 40×ResBlock(BN/Mish/Conv×2 + ChannelAttention) → Conv→32 → Linear→1024
                 │ φ (1024)
                 ▼
        DQN 头: Linear(1024→1+46)，Q = v + a − mean(合法 a)，非法位 −inf
                 │ argmax（默认贪心；可选玻尔兹曼采样）
                 ▼
        action id (0..=45) ──► mjai 事件（dahai/chi/pon/kan/reach/hora/…）+ meta(q_values, mask_bits, …)
```

## 1. 观测编码（libriichi，Rust）

`PlayerState::encode_obs(version, at_kan_select) -> (obs, mask)`，v4 形状 **(1012, 34)**：
1012 通道 × 34 牌位（万筒索 1-9 + 七种字牌）。当前**行动玩家视角**，只有可见信息。通道族：

- 手牌（每张一通道 ×4）+ 3 个赤牌标记；
- 四家点数（归一化）、顺位 one-hot、局数、本棒/供托（RBF 编码）、场风自风；
- 宝牌指示牌；
- 自家河：**前 6 + 后 18**（比中间重要），每项带细节通道（立直锁定、摸切、被鸣走等）；
  他家河同样式；v3 起再加一条"自家河按新旧指数衰减"的汇总通道；
- 各家副露（fuuro）概览；
- **v4 新增 "sp" 工程特征**：libriichi 穷举搜索算出的向听/听牌率/和率/切牌 EV 表
  （覆盖未来 ~17 巡）。这是 v4 强度的关键，也是编码"很吃 CPU"的原因（源码注释原话）。

训练时另有 oracle 观测（+217 通道，含三家暗牌），**推理不用**（`is_oracle=False`）。

`mask`：46 维 bool，由规则引擎判定当前合法动作。**合法性本身是规则算的**——但它只是
网络的约束，不产生选择。

## 2. 网络（`mortal/model.py`）

三个组件，推理只用到前两个（`mortal.py` 只加载 `mortal` + `current_dqn`）：

| 组件 | 结构 | 备注 |
|---|---|---|
| **Brain** | stem `Conv1d(1012→192,k=3)` → 40 × ResBlock → `Conv1d(192→32,k=3)` + Mish + Flatten + `Linear(32×34→1024)` | 10.8M 参数。ResBlock 为 pre-activation：`BN→Mish→Conv(k3)→BN→Mish→Conv(k3)` + **ChannelAttention** + 残差相加 |
| **DQN 头** | 单层 `Linear(1024→1+46)`，拆 v(1)/a(46) | 决斗式：`Q = v + a − mean(a[合法])`，非法位填 −inf。仅 48K 参数 |
| AuxNet | `Linear(1024→Σ辅助维度)` | 番数分布等辅助监督头，**只在训练用** |

ChannelAttention 是 SE 结构：对 34 轴做 avg-pool 与 max-pool，过共享 MLP，sigmoid 门控各通道。

另有两个训练专用件：v1 时代 Brain 尾部的 VAE 隐变量（mu/logsig，v4 已删）；**GRP**
（独立小 GRU，输入 [局数,本棒,供托,四家点数] → 24 种最终顺位排列 → 4×4 顺位概率矩阵，
RL 里做 reward 整形）。GRP 不参与动作选择。

## 3. 动作空间（`libriichi/src/consts.rs`，ACTION_SPACE = 46）

| id | 含义 | 解码成的 mjai 事件 |
|---|---|---|
| 0–33 | 打 34 种牌 | `dahai`（Rust 侧自动算 tsumogiri 标记） |
| 34–36 | 打红 5万/5筒/5索 | `dahai` |
| 37 | 立直 | `reach` |
| 38–40 | 吃（低/中/高嵌张位） | `chi`（consumed 自动推导，含赤牌替换） |
| 41 | 碰 | `pon` |
| 42 | 杠 | 按合法性自动分派 `daiminkan`/`ankan`/`kakan` |
| 43 | 和（自摸/荣和） | `hora` |
| 44 | 九种九牌 | `ryukyoku` |
| 45 | 跳过 | `none` |

**kan select 双推理**：暗杠/加杠有多张可选时，引擎会额外编码一份 `at_kan_select=true`
的观测再推理一次，此时动作 0–36 的语义变成"杠哪张牌"；结果挂在 `meta.kan_select` 里。

## 4. 动作选择与解码

1. 决策点到达 → `encode_obs` 出 (obs, mask)（+可选 kan-select 观测），rayon 并行编码；
2. `MortalEngine.react_batch`（Python，`torch.inference_mode`）：`φ=brain(obs)`，
   `Q=dqn(φ,mask)` → 46 个 Q 值；
3. **选择**：默认 `argmax`（纯贪心）。配置 `boltzmann_epsilon>0` 时以 ε 概率从
   `softmax(Q/温度)` 按 nucleus top-p 采样——这是 RL 自对弈/在线探索用的，
   `mortal.py` 默认 ε=0，即**对局推理永远贪心**；
4. Rust 侧 `match action` 逐 id 解码（§3 表），每支带 `ensure!` 合法性兜底；
5. 输出 `EventExt { event, meta }`。

## 5. rule-based 护栏（仅两处，都不是决策者）

**① quick_eval —— 跳网络的捷径（性能优化）**
条件：只能打牌、候选牌恰一张、且无立直/和牌/杠/流局可能 → 直接打那张牌，不推理。
动作被规则**强制唯一**，网络跑出来也必然同值——不算 rule-based 决策。`mortal.py`
中 `enable_quick_eval = not review_mode`，正常对局恒开。

**② rule_based_agari_guard —— 只管"末位见逃"的一条规则（默认开启！）**
`mortal.py` 写死 `enable_rule_based_agari_guard = True`。逻辑
（`libriichi/src/state/agent_helper.rs` 的 `rule_based_agari`）：

- 非最后半庄局 / 自己是亲家 / 不是末位 → 恒"该和"，护栏不干预；
- 唯一真正管事的场景：**all-last + 自己是末位子家**。算本次和牌的**最大理论点数**
  （枚举最优里宝牌排列），模拟和牌后四家点数；若"和了也逃不出末位、也保不住西入"
  → 判定见逃更优；
- 执行方式：网络选了 43(hora) 但规则说 no → `Q[43]` 置最小值，**重新 argmax 取次优**。
  它只能否决"和牌"，不能指定打哪张、鸣不鸣。

> 教程视角：这是整个 Mortal 唯一一处"白纸黑字的规则否决了 AI"，天然讲解素材
> （"AI 想和，但规则引擎算出和了也是 4 位，强制见逃"）。教程的 bot 参照
> （`src/bot/replay.rs::classify_reaction`）已把两类 rule 旁路标记出来：
> quick_eval＝`rule_forced`（无概率分布），agari guard＝`rule_overridden`
> （贪心首选非榜首即被否决），被选行带 `chosen` 标记。

## 6. meta 线格式（mjai.app 公共协议）

每个决策的反应带 `meta`（`gen_meta`，Rust）：

- `q_values`：**仅合法动作**的 Q 值，按动作 id 升序压缩排列；
- `mask_bits`：46 位掩码，`q_values[i]` 对应第 i 个置位 id——两者必须配套读；
- `is_greedy` / `shanten` / `at_furiten` / `eval_time_ns` / `batch_size`；
- `kan_select`：嵌套 meta（§3）。

**Q 值 ≠ 概率**。本仓库包装层 `mjai_bot/mortal/bot.py` 的 `build_show` 对合法 Q 值做
softmax 得候选概率，翻成 HUD 的 `meta.show` 卡片（"Pass 87% / Pon 13%"式）。教程侧
（D-0012 §2）的 `prob` 语义：live 深讲 = 该 `meta.show` 真 bot 概率；复盘 = 引擎份额近似。

## 7. 源码索引

Mortal 检出内（相对 `mortal_dir`，均为 Mortal 仓库路径）：

| 文件 | 内容 |
|---|---|
| `mortal/model.py` | Brain / DQN / AuxNet / GRP 定义（§2） |
| `mortal/engine.py` | `react_batch` 前向与动作选择（§4.2-3） |
| `mortal/mortal.py` | 引擎入口：加载权重、`enable_rule_based_agari_guard=True` |
| `libriichi/src/consts.rs` | ACTION_SPACE=46、obs_shape(v4)=(1012,34) |
| `libriichi/src/agent/mortal.rs` | 批推理调度、动作 id→事件解码、quick_eval、agari guard 执行、gen_meta |
| `libriichi/src/state/agent_helper.rs` | `rule_based_agari`（all-last 见逃判断，§5②） |
| `libriichi/src/state/obs_repr.rs` | 观测编码（§1） |

本仓库：`mjai_bot/mortal/bot.py`（协议代理 + meta.show 翻译）、`src/bot/README.md`
（subprocess 边界与 BotResponse.meta 约定）、[D-0010](../DECISIONS.md)（包装入库/权重忽略）、
[D-0012](../DECISIONS.md)（M4 mortal 分歧挖掘的推迟记录）。
