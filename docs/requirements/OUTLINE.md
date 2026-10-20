# dshg 需求纲要（一页纸）

- 版本：v0.6 草案
- 日期：2026-10-20
- 状态：**范围升级**——总体需求改为"完全复刻 dsh-tauri 桌面端的功能"（见
  [design/PARITY-TARGET.md](../design/PARITY-TARGET.md)），此前"MVP 子集"的范围与相关决策作废，
  需按 §10 重新确认；本阶段**只出文档，不写代码**

## 1. 定位

**dshg 是 dsh-tauri 的原生复刻**：Rust + GPUI + GPUI Kit，目标功能全量对齐
`dsh-tauri/deepseek-harness-desktop`（桌面端），分阶段实现；不重写 dsh 内核，
也不复用 Tauri/React 代码与 UI 资产。

一句话：**用原生桌面技术重新实现 dsh-tauri 的完整体验，而不是先做一个子集。**

## 2. 为什么做

- dsh 目前的主要形态是 Web UI（`dsh --profile web`，默认 `127.0.0.1:3080`），
  桌面侧已有 `dsh-tauri`（Tauri + WebView 包同一套前端）。[假设] 主要痛点是 WebView 的
  内存/渲染性能、系统集成（菜单、快捷键、通知、拖拽、辅助功能）与离线/多引擎管理体验。
- GPUI Kit（Longbridge，Apache-2.0，v0.7.x，75+ 组件）已提供原生高刷渲染、虚拟化列表/表格、
  Markdown 与代码编辑器、Dock 布局、主题、AccessKit 可访问性、无头 UI 测试，
  正好覆盖"信息密集型 coding agent"的界面需求。
- 目标不是"再包一层 WebView"，而是验证 GPUI 做复杂 agent 客户端的工程可行性。

**非目标（本纲要阶段）**：重写 agent loop / 工具 / 沙箱 / 插件运行时；像素级复刻 Web UI；移动端；
多用户 SaaS 形态。

## 3. 目标用户与核心场景

| 用户 | 场景 |
| --- | --- |
| 个人开发者（主力） | 本地仓库里开长会话写代码；并行多个会话；在本地即时处理审批 |
| 同一台机器的多项目使用者 | 多仓库切换，会话与 workspace 绑定清晰 |

不做远程/局域网连接（已确认）：用户在本机运行 dsh，dshg 只连本机 loopback。

关键场景：

- **S1 首次连接即聊**：打开 app → 粘贴 `dsh web` 打印的 URL（之后复用已存 cookie）→ 选 workspace →
  发第一条消息（引擎由用户启动）。
- **S2 长会话流畅**：1 万+ 事件的历史会话滚动、折叠、搜索不卡顿。
- **S3 审批可控**：工具调用需审批时，通知 + 一键批准/拒绝，且能追溯"谁批了什么"。
- **S4 工具结果可读**：bash 输出、diff、文件读写、子 agent、workflow 以结构化卡片呈现。
- **S5 会话可恢复**：崩溃/重启后恢复会话与草稿。

## 4. 形态与平台

- 形态：原生桌面应用（单窗口 + Dock 面板布局），无浏览器依赖。
- 平台：**macOS 先行，Windows / Linux 作为 P4 目标**（待确认，见 PARITY-TARGET §6）。
- 不做：Web 版、TUI；移动端 App 是否复刻待定；不再排除远程/局域网（旧决策作废）。

## 5. 架构主张

**GUI = dsh 的前端客户端；引擎 = 管理式运行时（默认）或用户已有的 dsh（可选 attach）。**

> 旧决策"只 attach、不内嵌"已被"完全复刻"覆盖：dsh-tauri 会下载并管理 Node/dsh、切换核心。
> 建议改为"默认管理式 + 可选 attach"（待确认），M0 的 attach 作为过渡与调试入口保留。

- 引擎连接：用户自行运行 `dsh --profile web`，dshg 通过 dsh **Client API（HTTP + WebSocket，
  Typert Remote gateway）** 连接，协议与官方 Web 前端一致。
  - **认证**：dsh 启动时打印带一次性 `?token=` 的 URL，用 `GET /?token=…` 换签名 cookie，
    之后 `/api/*` 与 WS 升级都只认 cookie；**没有查询 token 的 API**。
    因此首连 UX 是"粘贴 `dsh web` 打印的 URL"，cookie 存 Keychain（30 天）。
  - 连接管理：设置里可改地址；启动时探测，未连接时给引导（复制启动命令、重新粘贴 URL、重试）；
    断线自动重连；协议无版本协商，端点缺失时按"能力缺失"降级并提示。
  - 安全边界：只连本机 loopback（dsh 默认禁止 `0.0.0.0`）；不做远程 attach。
- 为什么不走另外几条路（详见 `docs/design/INTEGRATION-MODES.md`）：
  - `headless`：一次性任务，无追问/审批/会话列表；
  - `dsh-sdk-protocol`（stdio JSON-RPC）：只有 `initialize` / `session/prompt` / `shutdown`
    三个请求，撑不起完整 GUI；
  - `dsh-acp`：标准协议且有权限请求，但 automation-only、只发已提交消息（无 token 级流式），
    且没有 DSH 的 plan/todo/文件/设置/插件呈现；
  - 三者都是 stdio，必须由 dshg spawn 引擎，与"attach 已运行实例"冲突。
- 协议类型：从 dsh 的 TS 类型/协议定义生成 Rust 类型（或手写 + 契约测试），
  以官方 Web 前端的 `dsh-client-ui-*` 包行为作为参考实现。
- 引擎分发：**首版不内嵌**（已确认 attach 模式）。若后续做内嵌，再评估自带 Node 运行时
  （当前 node_modules 441MB、Node 运行时 191MB）与依赖用户 Node 的取舍。

## 6. 功能范围

| 模块 | MVP（首版必须） | 后续 | 不做 |
| --- | --- | --- | --- |
| 会话 | 新建/切换/列表/恢复/重命名/归档；事件流式渲染 | 搜索、fork、导出 | — |
| 消息 | Markdown、代码高亮、流式输出、复制、图片附件、@文件引用 | 富文本引用、语音输入 | — |
| 工具卡片 | bash/read/write/edit/glob/grep/web/todo/ask_user 的结构化渲染；原始 JSON 兜底 | 子 agent、workflow、deliverables 专用视图 | 自研工具 |
| 审批 | 权限预设、审批弹窗、批准/拒绝/记住、批量队列 | 审批历史与审计 | — |
| 输入 | 多行编辑器、@ 引用、/ 命令、发送/停止、粘贴图片 | 拖拽文件、快捷键自定义 | — |
| 侧栏 | 文件树 + 只读预览、diff 变更 | 终端（PTY）、文档预览 | — |
| 设置 | 模型/Provider/API Key、shell、沙箱、主题、语言、**插件启用/禁用** | 插件配置详情、技能管理、agent loop 细项 | — |
| 其他 | 通知、快捷键、单实例、崩溃恢复 | jobs、plan mode、goal、schedule、workflow | 移动端、Web 版 |

说明：本表只是 **P0 阶段**范围；全量功能（终端、插件市场、运行时管理、Profile、远程、宠物、
个性化、更新等）见 [design/PARITY-TARGET.md](../design/PARITY-TARGET.md)。旧"MVP 不含终端
与文件编辑"的边界仅适用于 P0；文件仍保持只读（dsh 无写 API），终端在 P2 纳入。

## 7. 非功能需求

- 性能：冷启动到可交互 < 1s（不含引擎启动，引擎由用户运行）；连接后首屏会话 < 300ms；
  会话切换 < 200ms；10k 事件滚动 ≥ 60fps；常驻内存 < 300MB。
- 可访问性：AccessKit 角色/名称/状态完整；全键盘可达。
- 国际化：zh-CN / en，跟随系统，可覆盖。
- 安全：只连本机 loopback；会话 cookie 存 Keychain；启动 token 不落盘；日志脱敏。
- 打包：macOS 签名+公证、Windows 签名、自动更新；崩溃上报默认关闭/可退出。
- 质量：gpui_kit::test 无头 UI 测试 + 协议 mock server 契约测试 + 真机冒烟。

## 8. 协议与版本策略

- 锁定 dsh 版本（当前 `@deepseek-ai/dsh@0.2.0-rc.2`，RC 阶段，"会有破坏性变更"）。
- 兼容策略：适配层隔离协议差异；每次升级跑协议契约测试与人工冒烟清单。
- 与 Web UI 的行为差异集中记录（`docs/design/PARITY.md`），避免"悄悄不一致"。

## 9. 风险

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| dsh 客户端协议未公开、RC 期频繁变更 | 高 | 协议适配层 + pin 版本 + 契约测试 |
| **启动 token 无查询 API**，只能从 dsh stdout 拿 | 高 | 首连 UX 定为"粘贴 URL"；cookie 缓存降低重复成本 |
| Rust 侧需自维护 HTTP+WS RPC 客户端 | 中 | 先做协议文档与最小连通性验证（暂不写产品代码） |
| 后续若内嵌引擎：Node 运行时体积/安全更新 | 中 | 首版不内嵌；需要时再评估双模式 |
| GPUI/GPUI Kit 在 Linux/Wayland、Windows IME 的成熟度 | 中 | 首版收敛到 macOS，后续里程碑实测 |
| 审批/权限 UI 与官方不一致导致误操作 | 中 | 以官方前端为参考，差异评审 |
| 官方 Web UI 迭代快，功能追赶 | 中 | MVP 边界 + 路线图对齐 |

## 10. 决策与待确认问题

**已被"完全复刻"覆盖、需重新确认**（2026-10-20 旧结论）：

| 旧结论 | 现状 | 建议 |
| --- | --- | --- |
| 只 attach、不内嵌 | 与 dsh-tauri 的管理式运行时冲突 | 默认管理式 + 可选 attach |
| 不做远程/局域网 | dsh-tauri 有 LAN/QR/隧道 | 桌面端远程纳入 P3，移动 App 待定 |
| 不做终端、不做文件编辑 | 终端在 dsh-tauri 中存在 | 终端 P2；文件保持只读 |
| 插件仅启停 | dsh-tauri 有完整插件管理 | P2 完整管理 |
| 首版仅 macOS | dsh-tauri 三平台 | macOS 先行，P4 三平台 |
| MVP 不做宠物/个性化 | dsh-tauri 有宠物与 8 调色板 | P3，可后置 |

仍然有效：

- 不改 dsh 源码、不 fork；不重写 agent 内核；
- 与官方 Web UI 长期并存、不替代（复刻对象是 dsh-tauri，不是 Web UI）；
- 不做像素级 UI 复刻（功能与流程对齐即可，待确认）。

待拍板问题见 PARITY-TARGET §6（引擎模型、平台、移动端、远程、宠物、验收口径）。

## 11. 下一步（只出文档，不写代码）

1. 确认 [PARITY-TARGET.md](../design/PARITY-TARGET.md) §6 的 6 个问题（引擎模型、平台、移动端、
   远程、宠物、验收口径）。
2. 按新范围重写 SPEC 的范围与里程碑（旧 SPEC 的架构仍适用，M0 仍是正确的第一步）。
3. 协议实测（`VALIDATION.md`）不变，仍是 M0 的前置。
4. 并行补齐 dsh-tauri 的功能细节（内置插件、Profile、运行时管理、宠物、更新器），
   逐项写详细设计；仍不写产品代码。
