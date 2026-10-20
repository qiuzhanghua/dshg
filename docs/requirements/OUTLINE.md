# dshg 需求纲要（一页纸）

- 版本：v0.5 草案
- 日期：2026-10-20
- 状态：方向已确认（首版仅 macOS、attach 外部 dsh 引擎、与官方 Web UI 长期并存、
  GUI 内可启停插件、不做远程连接、MVP 不含终端与文件编辑）；
  本阶段**只出文档，不写代码**；文中 `[假设]` 为未验证前提，`[待定]` 为仍需拍板的决策

## 1. 定位

**dshg 是 DeepSeek Harness（dsh）的原生桌面 GUI 客户端**：Rust + GPUI + GPUI Kit，
跨平台；只负责"界面 + 本地引擎连接"，agent 的模型调用、工具、沙箱、会话仍由 dsh 引擎提供。
**与官方 Web UI 长期并存，不替代它。**

一句话：**用原生桌面 UI 体验 dsh，而不是重写 dsh。**

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
- 平台：**首版仅 macOS**（已确认）。Windows / Linux 作为后续里程碑：GPUI Kit 声称三平台 + WASM，
  Linux 的 IME/Wayland 与 Windows 的 IME 需在对应里程碑实测。
- 不做：Web 版、TUI、移动端、远程/局域网连接（首版及可见版本）、多引擎实例管理。

## 5. 架构主张

**GUI = dsh 的前端客户端；引擎 = 用户自行运行的 dsh 进程（首版只 attach，不内嵌）。**

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

MVP 边界（已确认）：**不含内置终端面板，不含文件编辑/保存**（文件只读预览 + diff 查看）。

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

已确认（2026-10-20）：

- 定位：**与官方 Web UI 长期并存，不替代；不改 dsh 源码、不 fork**
- 首版平台：**仅 macOS**
- 引擎：**只 attach 外部 dsh 实例**，不内嵌
- 插件：GUI 内可**启用/禁用**；技能先只展示
- 远程连接：**不做**
- MVP：**不含终端面板、不含文件编辑/保存**
- 品牌：**暂不考虑**（默认沿用 Apache-2.0，签名/公证随后再说）

暂无其他需要拍板的问题；实现细节在 SPEC 中给出。

候选（v2，未拍板）：是否提供"自带引擎 + ACP 轻量模式"作为独立入口（功能子集、零 token 配置），
评估见 `docs/design/INTEGRATION-MODES.md`。

## 11. 下一步（只出文档，不写代码）

1. `docs/design/SPEC.md` —— v0.2 已完成，待评审。
2. `docs/design/PROTOCOL.md` —— v0.1 已完成（250 行，方法/事件/审批/插件，带源码依据），待实测验证。
3. `docs/design/PARITY.md` —— v0.1 已完成，待按当前 dsh 版本复核。
4. 评审 SPEC / PROTOCOL / PARITY → 排 M0 任务；M0 前先做协议实测
   （`docs/design/VALIDATION.md`：token→cookie、WS upgrade、session/follow、审批往返、插件启停），
   仍不写产品代码。
5. 评审通过并完成实测后，再开始编码；M0 任务拆分见 `docs/design/M0-DESIGN.md`。
