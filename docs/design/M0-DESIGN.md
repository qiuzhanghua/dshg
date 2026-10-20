# M0 详细设计：骨架与连通

- 版本：v0.1
- 日期：2026-10-20
- 上游：[SPEC](SPEC.md)、[PROTOCOL](PROTOCOL.md)、[VALIDATION](VALIDATION.md)
- 范围：M0 = 窗口骨架 + 配置 + 认证引导 + 连接 + 只读会话列表。
  不含消息流（M1）、发送（M2）、审批（M3）、设置/插件（M4）。

## 1. 设计目标与约束

- **可演示**：粘贴 `dsh web` 的 URL → 列出真实会话；重启 app 复用 cookie 免粘贴。
- **可测试**：连接状态机、协议 codec、错误映射都有单元测试；UI 关键流程用 `gpui_kit::test`。
- **可演进**：M0 的模块边界就是后续 M1–M4 的边界；不因早期图快把网络逻辑塞进 view。
- **约束**：attach 模式（不 spawn 引擎）、仅 macOS、无远程、UI 线程不阻塞。

## 2. 工程结构

M0 用**单 crate + 明确模块**，M2 之后再按需拆 `dshg-core` / `dshg-ui`（现在拆只会增加跨 crate 的
生命周期摩擦）。

```
src/
  main.rs               # 入口：Application + gpui_component::init + Root
  app.rs                # Root entity：装配 store/ui/connection，持有全局 Context
  config.rs             # config.json（schema v1）读写、迁移
  secrets.rs            # Keychain 封装（cookie 读写删）
  error.rs              # AppError 分类 + 面向用户的文案映射
  i18n.rs               # zh-CN/en 词典与查表

  connection/
    mod.rs
    supervisor.rs       # 连接状态机（唯一有权改状态的地方）
    auth.rs             # token URL → cookie；过期/401 处理
    http.rs             # unary RPC（reqwest + 超时 + 错误映射）
    ws.rs               # /api/remote.mux 连接、Ping/Pong、断线通知
    mux.rs              # 逻辑流多路复用：open/item/end/cancel ↔ item/error/end
    envelope.rs         # client-request / server-response 编解码

  protocol/
    mod.rs
    methods.rs          # M0：session/list、settings/describe、pluginInventory/list（探测用）
    events.rs           # $events 帧解码（ready/emit/waterfall/cancel）

  store/
    mod.rs              # Store：连接快照 + 会话列表 + 最近错误
    sessions.rs         # SessionSummary 列表、选中项、排序
    connection.rs       # UI 可见的连接状态快照

  ui/
    root.rs             # 根视图：Onboarding 或 Shell 二选一
    onboarding.rs       # 粘贴 URL、校验、错误提示、重试
    shell.rs            # Dock 壳：侧栏 + 主区 + 状态栏
    session_list.rs     # 虚拟化列表 + 选中
    placeholder.rs      # M0 主区占位（M1 换成 transcript）
    settings.rs         # M0 仅：连接信息、主题、语言、诊断
    theme.rs            # 主题 token 绑定
```

依赖（锁精确版本，`Cargo.lock` 提交）：

```toml
gpui = "0.2.2"            # 由 gpui-kit 0.8.0 的依赖关系确定
gpui-kit = "0.8.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "cookie"] }
tokio-tungstenite = { version = "0.24", features = ["rustls-tls-webpki-roots"] }
url = "2"
async-channel = "2"
keyring = "3"
time = { version = "0.3", features = ["formatting"] }
uuid = { version = "1", features = ["v4"] }
```

## 3. 并发模型

```
GPUI 主线程（UI）                        后台 tokio runtime 线程
┌──────────────────────────┐            ┌─────────────────────────────┐
│ Root / views             │  Command   │ Supervisor                  │
│ Store（仅 UI 线程可变）   │ ─────────▶ │  ├─ Auth（token→cookie）     │
│ async_channel 接收泵      │ ◀───────── │  ├─ Http（unary RPC）        │
└──────────────────────────┘  Transport │  ├─ Ws（remote.mux）         │
                              Event     │  └─ Mux（逻辑流表）           │
                                        └─────────────────────────────┘
```

- UI 线程只做：读 Store、渲染、发 Command、处理 TransportEvent（在 `cx.update` 中）。
- 后台线程只做：网络、解码、状态机；**不直接触碰 UI 实体**。
- 通道：`async_channel::unbounded`。UI 侧用一个 `Receiver` 泵任务（`cx.spawn` +
  `background_executor`），每批事件合并后触发一次 `cx.notify()`，避免逐条重绘。
- 禁止：在 view 里 `await`；在锁内 `await`；后台线程持有 `Window`/`Context`。

## 4. 连接状态机

状态与触发（`supervisor.rs` 唯一入口 `Command`，唯一出口 `TransportEvent`）：

| 当前 | 触发 | 下一状态 | 动作 |
| --- | --- | --- | --- |
| Idle | 启动 | NeedsAuth 或 Probing | 有有效 cookie → Probing；否则 NeedsAuth |
| NeedsAuth | 用户提交 URL | Authenticating | 解析 token、`GET /?token=…` |
| Authenticating | 303 + Set-Cookie | Probing | 存 Keychain、带 cookie 探测 |
| Authenticating | 其它 | NeedsAuth | 报错（token 无效/网络） |
| Probing | unary 成功 | WsConnecting | 打开 `/api/remote.mux` |
| Probing | 401 / 403 / 拒绝连接 | NeedsAuth / Failed | cookie 失效、栅栏、引擎未启动 |
| WsConnecting | 收到 `$events` ready | Ready | 记录 clientId，拉 `session/list` |
| WsConnecting | 超时（15s）或关闭 | Reconnecting | 退避 |
| Ready | WS 关闭 | Reconnecting | 保留 UI，标记 stale |
| Reconnecting | 退避到点 | Probing | 重新探测 + 建 WS |
| Ready | 401 | NeedsAuth | cookie 过期，要求重贴 URL |
| 任意 | 用户点"断开/重试" | 对应状态 | 取消旧连接 |

退避：500ms → 1s → 2s → 4s → 8s → 10s 封顶，±50% 抖动；手动重试清零。
UI 只读快照：`{state, attempt, last_error, engine_authority}`。

## 5. 认证与密钥

- URL 解析：只接受 `http://127.0.0.1:<port>/?token=<t>` 或 `localhost`；
  拒绝非 loopback（M0 明确不做远程）。
- cookie 获取：手动跟随（`reqwest` 关闭自动重定向，读 `location` 与 `set-cookie`），
  避免依赖 cookie jar 的平台差异。
- 存储：Keychain 条目 `dshg/session-cookie`，值为 JSON：
  `{authority, name, value, expires_at}`。authority 不匹配（端口变化）视为无效。
- 过期：`expires_at` 到点或收到 401 → 清 Keychain、进 NeedsAuth。
- **不落盘**：token 只存在于内存与 URL 输入框，不写日志、不写 config。

## 6. 数据流（M0）

1. Supervisor → `TransportEvent::SessionsLoaded(Vec<SessionSummary>)`。
2. Store 更新列表（按 `updatedAt` 倒序），保留选中项（不存在则选第一个）。
3. Root 触发一次 `cx.notify()`；`session_list` 用虚拟化列表渲染。
4. 连接状态变化 → Store 快照更新 → 状态栏重绘。
5. 错误 → Store.last_error（带分类），UI 决定 banner / onboarding 文案。

M1 起，同一管道增加 `session/follow` 帧与 event log；M0 的 `TransportEvent` 枚举预留
`SessionEvent` / `GenerationReset` 变体。

## 7. 界面（M0）

```
NeedsAuth:
┌──────────────────────────────────────────────┐
│  dshg                                        │
│  连接 DeepSeek Harness                        │
│  [ http://127.0.0.1:3080/?token=…        ]   │
│  [ 连接 ]   启动命令：dsh --profile web      │
│  错误：…（401 / 403 / 连接被拒绝）            │
└──────────────────────────────────────────────┘

Ready（Shell）:
┌──────────────────────────────────────────────────────────┐
│ 顶栏：dshg · 已连接 127.0.0.1:3080 · 主题/设置             │
├──────────────┬───────────────────────────────────────────┤
│ 会话列表      │  （M0 占位：选择会话后显示标题/时间/状态）   │
│ 搜索框        │                                           │
│ [会话…]      │                                           │
├──────────────┴───────────────────────────────────────────┤
│ 状态栏：连接状态 · 会话数 · 最近错误（可展开）              │
└──────────────────────────────────────────────────────────┘
```

- Dock 可折叠/缩放，布局写入 config（`ui.window`）。
- 设置（M0 最小）：连接地址（只读）、重新认证、主题、语言、导出日志（脱敏）。
- 通知：M0 不做系统通知（M3 审批时再加）。

## 8. 错误分类与文案

| AppError | 触发 | 用户文案（zh） | 动作 |
| --- | --- | --- | --- |
| `EngineNotRunning` | 连接被拒绝 | 找不到 dsh，请先运行 `dsh --profile web` | 复制命令 / 重试 |
| `Unauthorized` | 401 | 认证已过期，请粘贴新的启动 URL | 打开输入框 |
| `TrustFence` | 403 | 请求被 dsh 的浏览器信任栅栏拒绝 | 检查地址/端口 |
| `EndpointMissing` | 404 | dsh 版本不匹配（缺少接口） | 显示版本建议 |
| `Transport` | 超时/断连 | 连接中断，正在重连（第 N 次） | 自动重试 |
| `Rpc` | `ok:false` | 引擎返回错误：{code} | 显示详情 |
| `Decode` | 字段/帧解析失败 | 收到无法识别的数据 | 记录并跳过（不崩溃） |
| `LocalIo` | 配置/Keychain | 无法保存本地设置 | 只读模式运行 |

## 9. 配置与本地状态

`~/Library/Application Support/dshg/config.json`：

```json
{
  "version": 1,
  "connection": { "baseUrl": "http://127.0.0.1:3080" },
  "ui": {
    "locale": "zh-CN",
    "theme": "system",
    "window": { "width": 1200, "height": 800, "dock": {} }
  },
  "recent": { "lastSessionId": null }
}
```

- 写入用临时文件 + rename，避免半写坏文件。
- 未知字段保留（前向兼容）；版本升级走显式迁移函数。

## 10. 测试方案

| 层 | 内容 | 工具 |
| --- | --- | --- |
| codec | envelope / mux 帧序列化、未知字段容忍 | 单元测试 |
| auth | URL 解析（合法/非法/多 token）、Set-Cookie 解析、过期判断 | 单元测试（固定字符串） |
| 状态机 | 上表每条迁移；退避时间序列 | 单元测试（注入时钟） |
| store | 列表排序、选中保持、错误覆盖 | 单元测试 |
| UI | Onboarding 校验、Ready 空态、错误 banner | `gpui_kit::test` |
| 端到端 | 真机 dsh：粘贴 URL → 列表；重启免认证；kill/restart | 手工清单（见 §12） |

Mock：M0 不做 mock server；网络相关以纯函数（解析/状态机）为主，真实引擎放手工冒烟。
M1 引入协议级 mock（录制帧回放）。

## 11. 任务拆分（M0）

| # | 任务 | 产出 | 验收 |
| --- | --- | --- | --- |
| T0 | 脚手架：Cargo、模块骨架、日志、i18n 初始化 | 能启动空窗口 | `cargo run` 出窗口，无 panic |
| T1 | config + secrets | 配置读写、Keychain 封装 | 单测 + 手工重启保留设置 |
| T2 | envelope + http | 带 cookie 的 unary 调用 | 对真机 `settings/describe` 成功 |
| T3 | auth 引导 | URL 粘贴、token 交换、cookie 存储 | 401/403/拒绝连接文案正确 |
| T4 | ws + mux | `/api/remote.mux` 连接、`$events` ready | 日志出现 ready/clientId |
| T5 | session list | 列表拉取、排序、选中、空态 | 显示真实会话 |
| T6 | shell/UI 打磨 | Dock、状态栏、设置最小页 | 布局持久化 |
| T7 | 测试与冒烟 | 单测覆盖上表；冒烟清单 | 全绿 |

粗估：单人 7–9 个工作日（不含协议实测返工）。

## 12. M0 验收清单（真机）

1. 未启动 dsh → 打开 app 显示引导与可复制的启动命令。
2. 启动 dsh → 粘贴 URL → 连接成功 → 列出真实会话（含空列表态）。
3. 重启 app → 不粘贴 URL，自动连上（cookie 有效）。
4. 杀掉 dsh → 状态栏进入重连；重启 dsh 后自动恢复。
5. cookie 过期（或换端口）→ 回到引导，文案为"认证已过期"。
6. 错误地址 / 错误 token → 文案与 §8 一致，不崩溃。
7. 窗口大小与 Dock 布局重启后保持。
8. 冷启动到可交互 < 1s；空闲内存 < 300MB；日志无 token/cookie。
9. 单测全绿；`gpui_kit::test` 覆盖 onboarding 与空态。

## 13. 风险与开放问题

- **GPUI Kit API 变动**：0.8.0 很新，开工前先用 story 示例做一次编译探针；锁定 patch 版本。
- **Keychain**：非签名二进制访问 Keychain 可能弹权限；准备"仅本次会话"降级（内存 cookie）。
- **cookie 与 WS upgrade**：以 VALIDATION V4 为准；失败则 M0 的连接层要先改（见 PROTOCOL R3）。
- **token URL 来源**：M0 只做"粘贴"；解析 dsh stdout 留到后续（需要 dshg 管进程，与 attach 冲突）。
- **虚拟化列表**：GPUI Kit 的 VirtualList 在动态行高下的表现需在 T5 验证；兜底为固定行高。
