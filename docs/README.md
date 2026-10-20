# dshg 文档

dshg：DeepSeek Harness（dsh）的原生桌面 GUI 客户端（Rust + GPUI / GPUI Kit）。
当前阶段：**只做文档，不写代码**。

| 文档 | 内容 | 状态 |
| --- | --- | --- |
| [requirements/OUTLINE.md](requirements/OUTLINE.md) | 一页纸需求纲要：定位、场景、范围、NFR、风险、决策记录 | v0.6，范围升级 |
| [design/SPEC.md](design/SPEC.md) | 设计规格：架构、选型、数据模型、界面、流程、里程碑、验收 | v0.2 草案，待评审 |
| [design/PROTOCOL.md](design/PROTOCOL.md) | dsh 客户端协议梳理（HTTP + WS、事件、审批、插件），250 行、带源码依据 | v0.1 完成，待实测验证 |
| [design/PARITY.md](design/PARITY.md) | 与官方 Web UI 的功能对照与差异 | v0.1 草案 |
| [design/VALIDATION.md](design/VALIDATION.md) | 协议实测计划：V1–V11 用例、命令、判据、记录模板 | v0.1，V1–V5 已执行 |
| [design/VALIDATION-RESULTS.md](design/VALIDATION-RESULTS.md) | 实测结果：认证/WS/ready 帧（V1–V5 PASS） | 2026-10-20 |
| [design/M0-DESIGN.md](design/M0-DESIGN.md) | M0 详细设计：工程结构、状态机、数据流、UI、任务拆分、验收 | v0.1，待评审 |
| [design/PARITY-TARGET.md](design/PARITY-TARGET.md) | **复刻目标**：dsh-tauri 功能全集、冲突决策、P0–P4 路线 | v0.1，待确认 |
| [design/FRAMEWORK-COMPARISON.md](design/FRAMEWORK-COMPARISON.md) | GPUI Kit vs egui 选型对比与切换触发条件 | v0.1 |
| [design/INTEGRATION-MODES.md](design/INTEGRATION-MODES.md) | 引擎接入方式：web / headless / sdk / acp 对照 | v0.1 |

## 已确认的关键决策

- 与官方 Web UI **长期并存**，不替代、不改 dsh 源码
- 首版**仅 macOS**
- 引擎**只 attach** 用户本机已运行的 `dsh --profile web`，不内嵌
- GUI 内可**启用/禁用插件**；技能先只读展示
- **不做**远程/局域网连接
- MVP **不含**内置终端与文件编辑（文件只读预览 + diff）

## 实现进度（M0）

| 任务 | 状态 |
| --- | --- |
| T0 脚手架（gpui-kit 窗口） | 完成 |
| T1 config + Keychain | 完成（config 单测；Keychain 待真机验证） |
| T2 envelope + unary HTTP | 完成（17 个单测） |
| T3 token→cookie 认证 | 完成（`dshg --probe <url>` 对真实 dsh 验证） |
| T4 WS `/api/remote.mux` + `$events` ready | 完成（`dshg --events` 对真实 dsh 验证） |
| T5 会话列表 + 引导 UI | 完成（已手工验收） |

M1（只读回放）：`session/follow` 快照、Markdown/工具卡片、assistant `text-delta` 流式、
follow 断线自动重连已完成；gap 修复（`session/page`）与虚拟化列表待做。

M2（对话）：Composer + `session/prompt` 已接（`--prompt` 验证 accepted）。

M3（审批）：`$events` 长连接、`approval/request` 解析、审批卡片（允许一次/拒绝）、
`$events/result` 回执已实现；**端到端未验证**（隔离 home 无模型凭据，无法触发工具调用）。

工具卡片：按 callId 配对调用/结果；bash/read/grep/glob/web 参数摘要；运行中/成功/错误状态；
`tool/result.meta` 原样附注。diff 的专用渲染仍未做。

工程：core 拆成 `src/lib.rs`（config/connection/protocol/secrets/transcript），
`cargo test --lib` 23 个测试全绿，解析层不再依赖 GPUI。

## 运行

```sh
# 真机验证（需要先启动 dsh web）
dsh --profile web --no-open            # 记下打印的 URL
cargo run -- --probe "<url>"            # 认证 + 会话列表
cargo run -- --events "<url>"           # WS 升级 + $events ready
cargo run                               # GUI：粘贴 URL → 连接 → 会话列表
```

## 下一步

1. 确认 [PARITY-TARGET 的 6 个问题](design/PARITY-TARGET.md#6-待拍板问题)
2. 按全量复刻范围重写 SPEC 的里程碑与范围
3. 执行 VALIDATION（V1–V11），结果写入 `design/VALIDATION-RESULTS.md`
4. 按 M0-DESIGN 排任务，开始编码
