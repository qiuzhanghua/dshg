# dshg 设计规格（SPEC）

- 版本：v0.2 草案
- 日期：2026-10-20
- 上游需求：[需求纲要](../requirements/OUTLINE.md)（v0.4）
- 状态：**范围已升级**——总体需求改为完全复刻 dsh-tauri，本文的"v1 范围/里程碑"以
  [PARITY-TARGET.md](PARITY-TARGET.md) 为准；架构与协议部分仍然有效。

## 1. 范围

**总体目标**：macOS 上的原生 dsh 客户端，**功能全量对齐 dsh-tauri**（见
[PARITY-TARGET.md](PARITY-TARGET.md)），按 P0–P4 分阶段实现。

**P0 起点**（本文与 M0-DESIGN 描述的部分）：用户运行 `dsh --profile web`，dshg 连接它，
提供连接引导、会话列表与只读回放；后续 P1 起补对话/审批/设置/文件，P2 引入管理式运行时、
插件管理、终端，P3 远程与个性化，P4 跨平台。

**非目标**：agent loop / 工具 / 沙箱 / 插件运行时的任何重写；dsh 源码修改；
像素级复刻 dsh-tauri；移动端 App（后续独立评估）。

## 2. 总体架构

```
┌──────────────────────────── dshg (Rust, GPUI) ────────────────────────────┐
│  UI 层（gpui + gpui-kit）                                                 │
│    Root / Dock 布局 · 会话列表 · Transcript · Composer · 审批浮层          │
│    设置 · 插件管理 · 文件预览 · 通知                                       │
├───────────────────────────────────────────────────────────────────────────┤
│  状态层（Store）                                                          │
│    SessionStore · EventLog · ApprovalQueue · WorkspaceStore · Settings     │
├───────────────────────────────────────────────────────────────────────────┤
│  协议层（dshg_protocol）                                                  │
│    RpcClient（unary）· StreamClient（WS）· 事件解码 · 重连/generation      │
├───────────────────────────────────────────────────────────────────────────┤
│  连接层（后台运行时线程：tokio + reqwest + tokio-tungstenite）             │
└──────────────────────────────────┬────────────────────────────────────────┘
                                   │ HTTP + WebSocket（loopback）
┌──────────────────────────────────▼────────────────────────────────────────┐
│  dsh engine（用户自行启动的 Node 进程，不由 dshg 管理）                     │
└───────────────────────────────────────────────────────────────────────────┘
```

设计约束：

- **单一数据源在引擎侧**。模型、密钥、插件配置都由 dsh 持有；dshg 只通过 API 读写，
  本地只存"连接地址、窗口布局、主题、语言、草稿"等客户端状态。
- **网络与 UI 解耦**。HTTP/WS 跑在独立的后台运行时线程上，通过 channel 向 GPUI
  entity 投递事件；UI 不直接 `await` 网络（避免阻塞渲染循环）。
- **协议层可替换**。协议类型集中在一个模块，适配层隔离 dsh RC 期的字段变化；
  所有解码处都要能容忍未知字段。

## 3. 技术选型

| 关注点 | 选择 | 说明 |
| --- | --- | --- |
| UI 框架 | `gpui` + `gpui-kit`（0.7.x/0.8.x，开工时锁定） | 原生高刷、Dock、虚拟化列表、Markdown、代码编辑器、主题、AccessKit；选型对比见 [FRAMEWORK-COMPARISON.md](FRAMEWORK-COMPARISON.md) |
| 异步/网络 | tokio（专用线程）+ reqwest + tokio-tungstenite | 与 GPUI executor 解耦；rustls，不依赖 OpenSSL |
| 序列化 | serde / serde_json | 容忍未知字段；协议类型与 TS 定义对齐 |
| 错误 | thiserror + 统一 `AppError` | 分类：连接/协议/引擎返回/本地 IO |
| 日志 | tracing + 文件滚动 | 脱敏后落 `~/Library/Logs/dshg/` |
| 配置 | JSON（serde） | `~/Library/Application Support/dshg/config.json` |
| 密钥 | 不落地 | 由引擎管理；GUI 只提交到引擎 API |
| 测试 | 单元（codec/store）+ `gpui_kit::test` 无头 UI + mock engine | 真机冒烟另列清单 |
| 许可 | Apache-2.0 | 与 dsh、gpui-kit 一致 |

## 4. 协议接入

方法级清单见 [PROTOCOL.md](PROTOCOL.md)（基于 dsh 0.2.0-rc.2 源码，未运行验证）。关键事实：

- 启动：`dsh --profile web [--host] [--port] [--trusted-host] [--no-open]`。
- **认证**：进程级 launch token → 签名 cookie。
  1. dsh 启动时打印 `dsh web: http://127.0.0.1:<port>/?token=<t>`，token 只在进程内存，**没有查询 API**；
  2. `GET /?token=<t>` 换 `303 + Set-Cookie`（HttpOnly、SameSite=Strict、无 Secure、默认 30 天）；
  3. 此后所有 `/api/*` 与 **WS upgrade** 只认 cookie；没有 loopback 免认证。
  dshg 的 UX 因此是**粘贴 dsh 打印的 URL**（或 token），cookie 存 Keychain，过期/端口变化时重新粘贴。
- **Unary RPC**：`POST /api/<ns>/<method>`，`Content-Type: application/json`，请求体
  `{type:"client-request", rpcId, method, payload:{args:{<形参名>:值}}}`；响应
  `{type:"server-response", rpcId, result:{ok:true,value}|{ok:false,error:{code,message,details}}}`。
  业务失败仍是 HTTP 200；未认领端点回 404（两种失败语义要分开处理）。
- **流**：单条 WebSocket `/api/remote.mux` 复用多条逻辑流，只收文本帧；C→H `open/item/end/cancel`，
  H→C `item/error/end`，均带 `streamId`；逻辑流用 endpoint 区分。
- **事件**：Web 侧没有 `session.event` / `session.status`（那是 stdio SDK 的命名）。实际是
  27 项转发事件（`emit` + `approval/request`、`user-questions/request` 两个 **waterfall**）
  + 各域流帧；会话内容走 `session/follow` 的 `snapshot/records/assistantStream` 帧。
- **审批**：waterfall 帧到达，`POST /api/$events/result` 应答；可选结果只有
  `allowed-once | rejected | cancelled | unavailable`，**没有 always-allow**；
  持久策略属于 Host 侧 `permission-presets`。
- **插件启停**：`pluginInventory/list` + `pluginManager/setPluginEnabled`（以及 bundle 级），
  返回值 `ChangeResult.application ∈ {applied, restart-required, overridden, failed, cancelled}`，
  **必须读返回值**，重启要求交给用户（attach 模式不代管进程）。
- **版本**：没有版本协商；只能做端点存在性探测，把 `gateway/method-unavailable` 当能力缺失降级。

dshg 的协议层职责：

1. 连接引导：token URL → cookie（Keychain）→ 就绪探测。
2. 请求：unary RPC（超时、错误映射、404 与 `ok:false` 区分）。
3. 事件：waterfall 应答（审批/提问）、`session/follow` 帧解码、`connection/reset` 后重拉基线。
4. 重连：指数退避（500ms→10s，抖动）；换代后各域按 baseline 恢复，单向 emit 不重放。
5. 兼容：端点表按 dsh 包版本快照 + 严格白名单；未知错误码透传字符串，不猜语义。

### 4.1 为什么不是 headless / sdk / acp

dsh 还有 `headless`、`sdk`、`acp` 三个非 Web profile，但都不适合作为 dshg 的主接入面：

- 都是 **stdio** 模式：进程必须由 dshg 自己 spawn，与"attach 用户已运行的 dsh"冲突；
- `headless` 是一次性任务（无追问/审批/会话列表）；
- `sdk` 只有 3 个请求 + 4 个通知（无会话列表/审批/设置/插件）；
- `acp` 是最接近的替代（标准协议、权限请求、会话列表/恢复），但只发已提交消息
  （**无 token 级流式**），且没有 plan/todo/title/terminal/文件操作/设置/插件。

详见 [INTEGRATION-MODES.md](INTEGRATION-MODES.md)。

## 5. 数据模型（Rust 侧核心类型）

```rust
struct SessionSummary { id, title, workspace, updated_at, status, model, last_message_preview }
struct SessionEvent    { id, seq, kind, payload }        // kind: message/tool/approval/status/...
struct Message         { role, blocks: Vec<ContentBlock> }
enum ContentBlock      { Text, Image, Reasoning, ToolCall(ToolCall), ToolResult(ToolResult) }
struct ToolCall        { id, name, input, status }      // 按 name 选卡片渲染器
struct ApprovalRequest { id, session_id, tool, reason, options, deadline }
struct Workspace       { root, name, files_version }
struct PluginEntry     { id, name, version, enabled, config_schema? }
```

原则：

- 事件以**追加日志**形式缓存（按 session + seq 排序），支持"从头回放""跳到最新"。
- 未知事件类型保留原文，UI 以通用折叠块展示（不丢数据）。
- 所有 ID 用字符串（引擎生成），本地不做语义假设。

## 6. 界面与交互

### 6.1 窗口布局

```
┌──────────────────────────────────────────────────────────────────────┐
│ 顶栏：连接状态 · 当前 workspace · 模型 · 搜索 · 设置                  │
├───────────────┬──────────────────────────────────────┬───────────────┤
│ 会话列表       │ Transcript（虚拟化）                  │ 文件/变更      │
│  + 新建        │  消息 · 工具卡片 · 审批条 · 分隔线      │ 只读预览/diff  │
│               ├──────────────────────────────────────┤ （可折叠）     │
│               │ Composer：多行 / @ / / 命令 / 图片      │               │
└───────────────┴──────────────────────────────────────┴───────────────┘
```

Dock 面板可拖拽/缩放/折叠；布局持久化。默认一栏会话 + 一栏文件，文件栏可关。

### 6.2 组件映射（gpui-kit）

| 需求 | 实现 |
| --- | --- |
| 长 transcript | `VirtualList` / 自定义可变高度虚拟化；按事件分片渲染 |
| Markdown/代码 | gpui-kit Markdown + 高亮；大代码块折叠与"复制" |
| Diff | 只读 diff 视图（后续可换编辑器组件） |
| 文件树 | 虚拟化 Tree；只读打开到预览面板 |
| 输入框 | 多行编辑器 + 触发菜单（@ 文件、/ 命令） |
| 审批 | 顶部状态条 + 模态浮层 + 系统通知 |
| 设置/插件 | 表单 + 表格（虚拟化） |
| 主题/语言 | gpui-kit 主题 token；zh-CN/en |

### 6.3 工具卡片

按工具名注册渲染器：`bash`（等宽输出、折叠、退出码）、`read/write/edit`（路径 + diff/片段）、
`glob/grep`（文件/命中列表）、`web`（标题 + 链接）、`todo`（清单）、`ask_user`（问卷内联）、
`subagent/workflow`（树/阶段视图，MVP 先用通用卡片）。未知工具回落到"JSON + 复制"。

## 7. 关键流程

### 7.1 启动与连接

1. 读本地配置：地址、Keychain 里的会话 cookie、上次的 workspace 与会话。
2. 若 cookie 有效：直接就绪探测（`settings/describe` 之类的轻请求）→ 建 WS → 等 `$events` 的
   `ready` 帧。
3. 若没有 cookie 或已过期：首屏要求**粘贴 `dsh web` 打印的 URL**（含 `?token=`）；
   `GET /?token=…` 拿 `Set-Cookie` → 存 Keychain → 继续第 2 步。
4. 成功：拉 `session/list` + `settings/describe` + `pluginInventory/list`；恢复上次会话与布局。
5. 失败：按状态码给引导——401（cookie 失效/需重贴 URL）、403（Host/Origin 栅栏）、
   连接拒绝（引擎未启动）、404（端点不存在，可能 dsh 版本不匹配）。
   同时提供"复制启动命令"和"重试"。

### 7.2 发送一轮消息

1. Composer 提交 → 本地插入 pending 消息 → 调 `prompt` 类接口。
2. 引擎回执（消息 id）→ 状态置 running；事件流持续到达 → 逐条追加、按帧合并重绘。
3. 遇到审批事件 → 进入审批队列，顶栏高亮；用户批准/拒绝 → 回执 → 继续。
4. 状态转 idle → 移除 pending 标记；若失败，消息标红并可重试。

### 7.3 断线与恢复

- WS 断开：状态栏显示"重连中（第 N 次）"，退避 500ms → 1s → 2s → … → 10s 封顶，带抖动。
- 重连成功：`$events` 新代 `ready` 帧 → 本地广播 `connection/reset`：
  全量缓存重拉；当前会话用 `session/follow` 的新 `snapshot` 重建，必要时用 `session/page`
  按 `throughSeq/beforeSeq` 修 gap。**单向 `emit` 事件不重放**。
- 期间用户操作：只允许编辑草稿；发送按钮禁用并提示。

### 7.4 插件启停

1. 设置 → 插件：`pluginInventory/list` 列出清单（名称/版本/状态/`fiberPhase`）。
2. 切换启用/禁用 → `pluginManager/setPluginEnabled`（或 bundle 级）→ 读返回的
   `ChangeResult.application`：
   `applied` 直接刷新；`restart-required` 明确标注并给"重启 dsh"指引（不代管进程）；
   `overridden` 说明被更高优先级配置覆盖；`failed` / `cancelled` 回滚开关并提示。
3. 插件**配置**走 `settings/*`，与启停分开。

## 8. 设置与本地持久化

| 数据 | 位置 | 说明 |
| --- | --- | --- |
| 连接地址、最近 workspace | `config.json` | 明文，可手工编辑 |
| 窗口布局、主题、语言、快捷键 | `config.json` | 版本化字段，向后兼容 |
| 草稿 | 按会话存 `config.json` 或独立文件 | 崩溃恢复用 |
| **会话 cookie** | **Keychain** | 签名 cookie，绑定 `host:port`；30 天有效；过期/端口变化需重贴 URL |
| 日志 | `~/Library/Logs/dshg/` | 滚动、脱敏 |
| 模型/密钥/沙箱 | 引擎侧 | GUI 经 API 读写，不重复存 |

## 9. 错误处理与诊断

- 统一错误分类：`EngineNotRunning` / `Unauthorized` / `TrustFence` / `Transport` / `Rpc` /
  `EndpointMissing` / `Decode` / `LocalIo`；UI 文案与"下一步动作"一一对应。
- 诊断页（设置 → 关于/诊断）：引擎地址、dsh 包版本、连接代数、最近错误、导出日志（脱敏）。
  注意：协议本身没有版本协商，版本只能从包/启动信息里读，不能依赖握手。
- 不做自动上报；崩溃采集默认关闭。

## 10. 性能预算

| 指标 | 目标 | 测量点 |
| --- | --- | --- |
| 冷启动到可交互 | < 1s | app launch → 首帧 |
| 连接建立 | < 500ms | 探测 + WS 握手（不含引擎启动） |
| 会话切换 | < 200ms | 点击 → 首屏事件渲染 |
| 10k 事件滚动 | ≥ 60fps | 连续滚动 10s 掉帧数 |
| 常驻内存 | < 300MB | 闲置 + 单会长会话 |
| 事件追加到渲染 | < 50ms | 单条事件 p95 |

## 11. 里程碑

| 里程碑 | 内容 | 出口标准 |
| --- | --- | --- |
里程碑与 P0–P4 的映射（完整功能清单见 [PARITY-TARGET §3](PARITY-TARGET.md#3-分期路线建议)）：

| 里程碑 | 对应阶段 | 内容 |
| --- | --- | --- |
| M0 骨架与连通 | P0 | 窗口 + Dock 壳、配置、连接探测、会话列表（只读）；详见 [M0-DESIGN.md](M0-DESIGN.md) |
| M1 只读回放 | P0 | Transcript 虚拟化、Markdown/代码、通用工具卡片 |
| M2 对话 | P0 | Composer、发送/停止、流式渲染、草稿 |
| M3 审批 | P0 | 审批队列/浮层/通知、权限预设、超时处理 |
| M4 设置/文件/插件 | P1 | 设置页、只读文件树、插件启停与清单 |
| M5 桌面产品化 | P2 | 管理式运行时、核心切换、插件市场、Profile、终端、托盘 |
| M6 远程/个性化 | P3 | LAN/QR/隧道、调色板/模糊/缩放、更新器 |
| M7 跨平台 | P4 | Windows/Linux 适配与发布 |

## 12. 验收标准（v1）

演示脚本（真机、真实 dsh）：

1. 引擎未启动 → 首屏引导；启动引擎 → 粘贴 `dsh web` 打印的 URL → 列出会话。
2. 选择一个 1 万+ 事件的会话 → 滚动流畅、工具卡片与代码块正确。
3. 发一条会触发工具调用的消息 → 流式输出 → 出现审批 → 允许一次 → 工具结果 → 完成。
4. 拒绝一次审批 → 会话给出拒绝结果，不卡死。
5. 断开引擎再恢复 → 客户端自动重连（或重新粘贴 URL）、会话状态一致、无重复消息。
6. 设置里切换主题/语言/模型 → 立即生效（模型以引擎确认状态为准）。
7. 禁用某插件 → 按 `ChangeResult.application` 呈现；需要重启时给出指引，重启后状态保持。
8. 性能指标达到第 10 节阈值（记录测量数据）。

## 13. 依赖与开放问题

实测计划见 [VALIDATION.md](VALIDATION.md)（V1–V11，含命令与判据）。

- **token / URL 获取**（最大阻塞）：launch token 只在 dsh 进程内存与 stdout，无查询 API。
  M0 前必须定 UX：只支持"粘贴 URL"，还是同时提供"解析 dsh stdout / 启动器集成"。
- **cookie 与 WS upgrade**：cookie 是否足以完成 WebSocket 升级，需实测（源码判据共用，
  但未运行验证）；过期/端口变化后的重连体验也要实测。
- **`session/follow` 帧序列**：`snapshot + surfaceOp + assistantStream` 的真实边界、
  重连 gap 修复，决定会话渲染正确性（PROTOCOL 已列必测点）。
- **审批 waterfall 往返**：`displayReason` 是否上线、`outcome` 实际取值、超时/取消后的
  session 行为（PROTOCOL §6、§9）。
- **插件启停生效方式**：`setPluginEnabled` 在真实 profile 上是 `applied` 还是
  `restart-required`；对运行中会话的影响。
- **文件预览能力边界**：`workspaceFiles/read`/`readBytes` 的大文件与二进制策略。
- **dsh RC 稳定性**：0.2.0-rc.2 起至少跟一个 minor；端点表按版本快照。
- **macOS 最低版本**：GPUI/GPUI Kit 实测后确定（暂定 macOS 13+）。
