# 贡献指南

本仓库是 [shinkuan/Akagi](https://github.com/shinkuan/Akagi)（雀魂 / 天凤 / 立直城市实时麻将 AI 助手）`v3` 分支的 fork，使命是在其上构建 **AI 麻将教程**。产品方向与里程碑见 [docs/ROADMAP.md](docs/ROADMAP.md)。

如果你是 AI 编码代理，请直接读 [AGENTS.md](AGENTS.md)——那是本仓库对代理的完整工作规则，与本文件互补。

## 环境准备

| 依赖 | 说明 |
|---|---|
| Rust stable | 后端 + 内置 bot |
| `protoc` | **必须在 PATH**，`build.rs` 用它编译 `src/bridge/majsoul/proto/liqi.proto`（Windows 上最常见 的构建失败就是缺它） |
| Node ≥ 20 + npm | 前端（Vite 8 要求） |
| Linux 额外 | WebKit2GTK 开发包（Tauri 依赖） |
| 打包发布时 | `bash scripts/fetch-runtime.sh` 填充 `runtime/`（Python + uv，供外部 bot） |

## 常用命令

```bash
cargo tauri dev                          # 一条命令起完整开发环境
# 或者手动：cd frontend && npm install && npm run dev（:1420）+ 另一终端 cargo run

cargo fmt --all -- --check               # 交付前必须全绿（同 CI）
cargo clippy --all-targets -- -D warnings
cargo test                               # 尽量全量；CI 最低跑 --lib 与集成编译检查
cd frontend && npm run lint && npm run build && npm run test
```

完整构建/打包流程见根目录 `README.md` 与 `scripts/README.md`。

## 分支与提交

- 主分支为 `v3`（与上游同名，保持合并历史干净）。功能分支 `feat/<scope>-<slug>`，修复分支 `fix/<issue>-<slug>`。
- 提交信息用 **Conventional Commits**（英文）：`feat(tutorial): add lesson renderer`、`fix(history): …`。scope 用模块名（`frontend`、`tutorial`、`history`、`majsoul`、`tenhou`、`riichi-city`…）。
- 版本号提升单独提交：`chore: update version number to x.y.z`。

## Pull Request 要求

PR 是本仓库的基本工作单元（上游 600+ 提交几乎全部经 PR 合入）。合并前自查：

1. `cargo fmt` / `cargo clippy -D warnings` / `npm run lint` / `npm run build` 全绿。
2. 行为改动附带测试（Rust 模块内或 `tests/`；前端 `*.test.ts(x)`）。
3. 改动了模块行为 → 同步更新该模块 `README.md`。
4. 前端新增用户可见字符串 → 四种 locale 的 i18n 资源全部补齐。
5. **改动了上游已有的文件 → 在 PR 描述中逐一列出并说明为什么非改不可**（fork 需要保持与上游可合并，见 AGENTS.md §2 规则 1）。
6. 影响架构 / 目录 / 依赖 / 产品方向的决策 → 先在 [docs/DECISIONS.md](docs/DECISIONS.md) 留痕。

## 同步上游

```bash
git remote add upstream https://github.com/shinkuan/Akagi.git   # 一次性
git fetch upstream && git merge upstream/v3
```

冲突处理原则：上游文件保上游，fork 改动重放；`docs/`、`src/tutorial/`、`frontend/src/components/tutorial/` 等 fork 专属目录不应产生冲突。

## 行为准则

教程项目的定位是教育：不增强代打能力，实时指导功能必须可关闭；尊重对局数据隐私（本地优先）；不提交密钥与不符许可证的代码（AGPL 的 Mortal 留在子进程边界之后）。
