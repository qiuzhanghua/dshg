# 引擎接入方式对比：web / headless / sdk / acp

- 版本：v0.1
- 日期：2026-10-20
- 问题：`dsh --profile web` 是 Web 技术栈，是否存在更适合 GUI 集成的 headless 方式？
- 结论：**v1 没有更好的替代**。web 是唯一同时满足"attach 已运行实例 + 流式 + 审批 + 完整功能"的接入面；
  headless / sdk / acp 都是 stdio 模式，必须由 dshg 自己 spawn 引擎，且各有能力损失。

## 1. 先澄清两个概念

1. **web ≠ 必须有浏览器**。`dsh --profile web --no-open` 就是纯服务器：只监听 loopback，
   不打开浏览器。所谓"headless 服务器"目前已经是 web profile 的一种用法。
2. **真正的非 Web 接入**在 dsh 里是另外三个 profile：`headless`、`sdk`、`acp`。
   它们都通过 stdin/stdout 通信，**进程必须由客户端拥有**（stdio 是进程私有的）。

## 2. 四种模式对照

| 维度 | web | headless | sdk | acp |
| --- | --- | --- | --- | --- |
| 传输 | HTTP + WebSocket（loopback） | 一次性 CLI，stdout/stderr | stdio JSON-RPC | stdio（ACP v1） |
| 生命周期 | 长驻，可 attach | 一次一任务，退出 | 长驻，GUI 必须 spawn | 长驻，GUI 必须 spawn |
| 与"attach 已运行实例" | ✅ | ❌ | ❌ | ❌ |
| 会话 | 列表/恢复/fork/搜索/元数据 | `--session-id` 续跑单会话 | 按 `sessionId`，无列表 | `session/list`、`session/resume`、`session/close`；无 fork/删除 |
| 流式粒度 | 事件流 + `assistantStream` 增量 | `--json` 事件流 | `session.event` 全量事件 | **仅已提交消息/思考**，无 token 级增量 |
| 交互审批 | waterfall，`allowed-once`/`rejected` | 无（按策略默认执行） | 无（按策略默认执行） | `session/request_permission`，一次性允许/拒绝 |
| 工具呈现 | 富：tool cards、diff、plan、todo、title | 无 | 原始事件 | 仅通用 tool lifecycle |
| 模型/推理强度 | settings + 凭证 | 继承配置 | `initialize` 时固定 | `session/set_config_option` |
| 插件启停 | ✅ inventory + enable/disable | — | — | — |
| 文件/workspace | workspace/files API | cwd | cwd | cwd（不支持 additional dirs） |
| 认证 | 一次性 token → cookie | 无 | 无 | 无（stdin 即信任边界） |
| 适合做 GUI 后端 | ✅ | ❌ | ❌ | ⚠️（轻量聊天可以，功能面不够） |

## 3. 为什么 headless / sdk / acp 不是"更好的集成"

### 3.1 headless：一次性任务，不是会话服务

官方定位就是脚本/CI：`dsh --profile headless "run the tests"`，跑完打印最终答案就退出。
有 `--json` 事件流和 `--session-id` 续跑，但**没有交互式追问、没有审批、没有会话列表**。
它可以作为 dshg 的"一次性任务"辅助功能，但不能当主界面后端。

### 3.2 sdk：协议太小

`dsh --profile sdk` 的 stdio JSON-RPC 只有 3 个请求（`initialize` / `session/prompt` / `shutdown`）
和 4 个通知（`session.event` / `session.status` / `subagent.started` / `subagent.finished`）。
没有会话列表、没有审批、没有设置、没有插件、没有文件树。做不了 dshg 的 MVP。

### 3.3 acp：最接近，但代价明确

`dsh --profile acp` 是最"标准"的轻量方案：ACP v1 + `session/list`/`resume`/`close`，
多会话、模型/推理强度、`session/request_permission`（一次性允许/拒绝）、
通用 tool lifecycle、上下文用量。用官方 `agent-client-protocol` 类库接，协议工作量比自研
HTTP/WS 小。

但它是**automation-only**，设计上就不带 DSH 的呈现层：

- 没有 token 级流式（只有已提交消息/思考）——对聊天体验是硬伤；
- 没有 DSH 的 plan / todo / title / terminal / 文件操作 / elicitation；
- 没有 fork、删除、additional dirs；
- 没有设置页、插件管理、workspace 文件树；
- 必须由 dshg spawn 引擎进程，与"attach 用户已运行的 dsh"的已确认决策冲突。

## 4. 结论与建议

| 目标 | 建议 |
| --- | --- |
| v1：attach 用户已启动的 dsh，功能对齐 Web UI | **保持 web 模式**（`--profile web --no-open`），这是唯一选择 |
| 想做"零配置、自带引擎"的桌面应用 | 届时候选 ACP：无 token、标准协议、审批内置；但要接受上面列的能力损失，并重新拍板引擎生命周期 |
| 想做远程/多客户端 | 只能 web（ACP 是 stdio，不适合远程） |
| 自动化/CI/快速任务 | headless（`--json`），作为 dshg 的侧功能而非主链路 |
| 想少写协议 | 只有 ACP 能明显省事，但省下的是协议工作量，换来的是功能面收窄 |

一句话：**web 的"重"换来的是能力和 attach；stdio 的"轻"要求 dshg 自己管进程并放弃大量功能。**
在"与官方 Web UI 长期并存、attach 外部实例"的既定方向下，web 仍是正确选择。

## 5. 对现有文档的影响

- SPEC §5 的接入决策不变；本文作为"为什么不走 headless/SDK/ACP"的依据。
- 新增一个待决问题（v2）：是否提供"自带引擎 + ACP 轻量模式"作为一个独立、功能子集的入口。
  如果做，需要重新定义插件/设置/文件面板的取舍，不能与 v1 的 web 模式共用一套 UI 假设。

## 6. 依据

- `dsh-headless` README：one-shot、`--json`、`--session-id`、不开端口。
- `dsh-sdk-app` / `dsh-sdk-jsonrpc-server` README：3 请求 + 4 通知，无审批。
- `dsh-acp` README：ACP v1 调用表、`session/request_permission`、明确列出的不支持项
  （load/fork/删除/additional dirs/modes/commands/plans/terminals/client fs/elicitation）。
- `dsh-web-app/lib/startup.js`：`--no-open` 只影响是否打开浏览器。
- 本机 `dsh --help`：示例含 `web`、`headless`、`sdk`、`acp`、`tui`（后者本安装未带 bundle）。
