# AGENTS.md — 本仓库的 Agent 工作规则

> 适用于在本仓库工作的所有 AI 编码代理（ZCode / Claude Code / Codex 等）。
> 人类贡献者请读 [CONTRIBUTING.md](CONTRIBUTING.md)；产品方向见 [docs/ROADMAP.md](docs/ROADMAP.md)。
> 本文件受版本管理（上游 Akagi 把 AGENTS.md 当私有文件忽略，本 fork 刻意反其道而行，见 `.gitignore`）。

## 1. 项目是什么

- 本仓库是 [shinkuan/Akagi](https://github.com/shinkuan/Akagi) `v3` 分支的 **fork**（remote `origin` = `HcPlu/mahjong_tutorial`）。
- 上游产品：面向雀魂 / 天凤 / 立直城市的**实时麻将 AI 助手**桌面应用（Rust + Tauri 2 后端，React 19 + TypeScript + Vite 前端）。它通过 MITM 代理或 CDP 抓取对局流量，镜像牌局状态，实时展示向听、待牌、和率、铳率与打牌建议。
- **本 fork 的使命：在上游能力之上构建 AI 麻将教程（AI-based tutorial）——用 AI 教人打麻将。** 上游已有实时分析引擎、bot 决策、完整对局历史（mjai 事件流），教程功能是对这些能力的重组与增强，而不是另起炉灶。路线图与里程碑见 [docs/ROADMAP.md](docs/ROADMAP.md)。

## 2. 硬性规则（违反即视为未完成）

1. **保持可同步上游。** 本 fork 长期跟踪 Akagi `v3`。新功能、新文件一律放进新目录或文件末尾追加，**不要重写、重排、重格式化上游已有代码**。确需修改上游文件时，改动要最小化，并在 PR 描述中列出「改动了哪些上游文件、为什么非改不可」。
2. **提交信息遵循 Conventional Commits（英文）**：`feat(scope):` / `fix(scope):` / `docs(scope):` / `test(scope):` / `refactor(scope):` / `chore(scope):`。scope 用模块名（`frontend`、`tutorial`、`history`、`majsoul`、`tenhou`、`riichi-city`…）。版本号提升是独立的一条 `chore: update version number to x.y.z` 提交。
3. **本地检查必须全绿再交付**（与 CI 完全一致，CI 会挡下任何警告）：
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cd frontend && npm run lint && npm run build   # build 含 tsc 类型检查
   ```
4. **行为改动必须带测试。** Rust 测试就近放模块内或 `tests/`（集成），前端测试与源码同目录（`*.test.ts(x)`，vitest）。
5. **改模块行为必须同步更新该模块的 `README.md`。** 本仓库的一大资产是每个 `src/<module>/` 下真正的设计文档（不变量、陷阱、扩展方法），让它过期等于破坏文档。
6. **前端用户可见字符串必须走 i18n**，四种 locale（`frontend/src/i18n/resources/{en,ja,zh-CN,zh-TW}.json`）都要补齐。`announcements/` 条目还要求日期严格降序。
7. **许可证边界不可越。** Mortal 等 AGPL bot 必须留在子进程边界之后（`mjai_bot/` + JSONL stdio 协议），其代码不得链接进主程序。不提交任何密钥、token、bot 权重之外的二进制大文件。
8. **教程功能的定位是教育。** 不开发新的"代打"（autoplay）能力或引擎（不做 MITM 帧注入式代打、不写新的点击自动化）；上游既有 autoplay 引擎可复用，允许为其提供便利 UI（如对局内开关，D-0011）；实时指导类功能必须可开关且默认不干扰对局（见 ROADMAP 非目标）。
9. **LLM 接入必须走抽象层**（见 §6），API key 永远不进仓库、不进日志。

## 3. 架构速览（改代码前必读）

数据主干（mjai 是全项目通用语言）：

```
capture (MITM/CDP) → bridge::<platform> → MjaiBus → game_state::tracker
    → PostTrackerBus → ┬→ analysis::runner → AnalysisBus          （非 AI 分析：向听/铳率/和率）
                        └→ bot::manager     → BotResponseBus        （AI 决策 + meta.show 候选列表）
    → ipc (Tauri 事件) → 前端 HUD / overlay 窗口
```

- 所有广播通道只在 `src/event_bus.rs` 定义，`src/lib.rs` 负责装配——加新通道先看这两个文件。
- **动手前先读对应模块的 `src/<module>/README.md`**，那里有不变量和陷阱。最深入的是 `src/bot/README.md`。

| 位置 | 职责 |
|---|---|
| `src/capture/` | 抓包后端：MITM 代理（hudsucker）/ Chromium CDP |
| `src/bridge/` | 平台协议 → `MjaiEvent`：majsoul（liqi protobuf）、tenhou（XML/JSON）、riichi_city（MITM + mlua） |
| `src/schema/mjai/` | `MjaiEvent` 枚举——全项目的通用事件语言 |
| `src/game_state/` | riichienv 驱动的实时牌局镜像与快照 |
| `src/analysis/` | 分析引擎（Go mahjong-helper 的 Rust 移植）：向听、待牌、和/听率、铳率、切牌搜索 |
| `src/bot/` | bot 管理器：native 内置 bot / MJOT 云推理 / 外部 mjai 子进程；购买与许可 |
| `src/autoplay/` | 把 bot 决策执行进真实客户端（CDP 模拟点击）——引擎不改；fork 侧只为其加对局内开关（D-0011） |
| `src/history/` | 对局历史持久化：`history/index.jsonl` + `games/<ulid>.mjai.jsonl` + 统计 |
| `src/inspector/` | 协议帧 / 事件 / bot 反应（含 meta）的广播，供 Inspector UI |
| `src/ipc/` | **唯一**接触 Tauri `AppHandle` 的模块：命令、事件转发、overlay 窗口 |
| `src/config/` | 类型化 `AppConfig`（TOML），保存走 `toml_edit` 保格式合并 |
| `native_bot/` | 内置纯 Rust CNN bot（子 crate，含训练脚本与嵌入式权重） |
| `mjai_bot/` | 外部 bot 插件接口：JSONL stdio + `manifest.toml`（作者指南见其 README） |
| `frontend/src/` | `routes/`（页面）、`tiles/`（仪表盘磁贴）、`stores/`（每域一个 Zustand store）、`i18n/`、`components/review/` |

与教程最相关的既有资产：`src/history/`（原始教材数据）、`src/analysis/`（可解释的数值依据）、`BotResponse.meta`（后端不解释、前端渲染的自由 JSON——天然的"讲解"通道）、Review 页（整局复盘，现为 MJOT 云端任务）、Inspector（全量事件流）。

## 4. 常用命令

前置依赖：Rust stable、**`protoc` 必须在 PATH**（`build.rs` 编译 liqi.proto）、Node ≥ 20（Vite 8）、Linux 上需 WebKit2GTK 开发包。打包前需 `bash scripts/fetch-runtime.sh` 填充 `runtime/`。

```bash
# 开发（一条命令；需 cargo install tauri-cli --locked）
cargo tauri dev

# 或手动两条终端
cd frontend && npm install && npm run dev    # Vite dev server :1420
cargo run                                    # debug 构建加载 devUrl

# 检查（交付前必须全绿，同 CI）
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cd frontend && npm run lint && npm run build

# 测试
cargo test                                   # 本地尽量全量（含 tests/ 集成测试）
cargo test --lib                             # 快速单元集（CI 最低要求）
cargo test --no-run --all-targets            # 集成测试编译检查（CI 最低要求）
cd frontend && npm run test                  # vitest

# 发布（打便携 zip 到 dist/）
bash scripts/fetch-runtime.sh
cargo tauri build --no-bundle
bash scripts/package-zip.sh <target-triple>
```

注意：裸 `cargo build --release` 必须加 `--features custom-protocol`，否则窗口会去连 dev server。

## 5. Fork 策略与决策管理

- **目录约定**：fork 新增的治理与文档放 `docs/`；教程后端代码放 `src/tutorial/`（建模块时配 `README.md`，遵守既有惯例）；教程前端放 `frontend/src/routes/Tutorial/`、`frontend/src/stores/tutorialStore.ts`、`frontend/src/components/tutorial/`。
- **决策必须留痕**：影响架构、目录、依赖、产品方向的决定，在 [docs/DECISIONS.md](docs/DECISIONS.md) 追加一条（格式见该文件），然后才动手。
- **同步上游**：`git remote add upstream https://github.com/shinkuan/Akagi.git && git fetch upstream && git merge upstream/v3`。合并冲突优先保上游、fork 改动重放；`src/tutorial/`、`docs/`、`frontend/src/components/tutorial/` 等纯 fork 目录不应有冲突。
- **分支模型**：`v3` 为主分支（与上游同名同历史），功能开发用 `feat/<scope>-<slug>`、修复用 `fix/<issue>-<slug>`，完成后合并回 `v3`。

## 6. AI 教程功能的落点（详见 ROADMAP）

- 后端：`src/tutorial/`——对局讲解生成、课程内容、LLM provider 抽象、与 `history`/`analysis`/`bot` 的对接。首发形态是**云端 OpenAI 兼容 API**（D-0007）；本地部署是"换 `base_url`"级别的备选，不是独立代码路径。
- 前端：教程路由 + `tutorialStore` + 讲解渲染组件（牌面用现成 `mahgen` Web Component）。
- LLM key 从 `AppConfig`（走 `toml_edit` 合并路径）或环境变量 `TUTORIAL_LLM_API_KEY` 读取；所有 LLM 调用要有超时、失败降级（没有讲解也比崩溃好）和成本意识（缓存、批量）。
- 教程数据（对局历史、学员进度）是用户隐私，默认本地存储；送云端 LLM 前要脱敏并在设置里可关。

**Agent 在 `src/tutorial/` 工作的注意事项**（违反任何一条都算未完成）：

1. **三区单向流不可绕过**（D-0007）：出网必经 `upload.rs`（consent 门控 + 脱敏），入网必经 `download.rs`（校验 + 反幻觉核对），落盘必经 `store.rs`。禁止在 provider / pipeline 里直连 reqwest 或直接写文件。
2. **consent 先于调用，缓存先于请求**：`upload_consent` 未开或 key 未配置时不得发起任何网络请求；调 LLM 前先查 `TutorialStore` 缓存（同一局复盘只花一次钱）。
3. **讲解数字的来源与校验分阶段**：D-0005 的长期原则是数字只能来自 `analysis` 引擎；**现阶段（D-0013，仓库主人 2026-10-11 决定）走信任直通**——LLM 输出原样保留，不逐项核对、不丢弃巡。严格校验器（`parse_review_doc*`/`parse_turn_doc`）保留在 `download.rs` 并持续测试，阶段结束切回即可。
4. **低耦合边界**：`src/tutorial/` 不得 `use crate::ipc`（方向只能是 ipc → tutorial）；脱敏逻辑在 `upload.rs` 自持一份；对 `history`/`analysis` 只读。**可以**依赖 `crate::bot` 的公开接口（`BotResponse`、`BotRunner`、`bot::replay` 等，仓库主人 2026-10-11 放开；bot 参照/候选概率类功能优先走 `bot` 的公开 API 而非复制其内部逻辑）。
5. **key 永不进日志、不进错误信息、不进仓库**；新增用户可见字符串走 i18n 四语言。

## 7. 陷阱清单（前辈用血换的）

- **`protoc` 缺失**是 Windows 上最常见的构建失败原因。
- overlay 窗口的三件套必须同步改：`src/ipc/overlay.rs` 的窗口 label ↔ `capabilities/overlay.json` ↔ tauri 的 `macos-private-api` feature 与 `app.macOSPrivateApi`。
- `config.toml` 保存必须走 `src/config/merge.rs` 的 `toml_edit` 合并路径（`OPAQUE_TABLES`），直接序列化会删掉用户手写的键。
- liqi.proto 由 CI（`auto-liqi.yml`）每日自动更新，手动改它会被覆盖；要改翻译逻辑去 `src/bridge/majsoul/` 的 mapper。
- 集成测试里 bench（`tests/analysis_bench.rs`）要 `--release` 才有意义。
- 交付前自查：新增文件是否被 `.gitignore` 误伤（上游 ignore 规则较激进，例如 `test`、`dist/`、`agent*.md`）。
- 教程产物落在 `<config_root>/tutorial/`（复盘缓存 `reviews/<游戏ULID>.json`），由 `util::resolve_dir` 解析，与 `history/` 平级；改存储格式时向后兼容旧 JSON（serde `#[serde(default)]`），别写迁移脚本。
