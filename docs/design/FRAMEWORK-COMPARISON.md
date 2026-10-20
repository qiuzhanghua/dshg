# 框架对比：GPUI Kit vs egui（面向 dshg）

- 版本：v0.1
- 日期：2026-10-20
- 结论：**dshg v1 继续用 GPUI Kit**；egui 作为"跨平台/Web 优先"或"GPUI Kit 卡住"时的备选方案。
- 事实基线（2026-10-20 查询）：[gpui-kit.com](https://gpui-kit.com/)、[crates.io gpui-kit](https://crates.io/crates/gpui-kit)、
  [egui README](https://github.com/emilk/egui)、[crates.io egui](https://crates.io/crates/egui)、
  [crates.io gpui](https://crates.io/crates/gpui)、[crates.io egui_kittest](https://crates.io/crates/egui_kittest)。

## 1. 一句话区别

- **GPUI Kit**：Zed 的 GPUI 之上的"应用工具包"——组件、Dock、Markdown、代码编辑器、主题、可访问性、测试
  都是现成的，偏"做产品"。
- **egui**：一个**立即模式 GUI 库**，不是框架。渲染与交互极简、跨平台/Web 一流，但
  Markdown、代码编辑器、Dock、虚拟化表格这些要自己拼或依赖社区 crate。

## 2. 版本与生态（事实）

| 项 | GPUI Kit | egui |
| --- | --- | --- |
| 最新版本 | `gpui-kit 0.8.0`（2026-10-19；站点仍标 0.7.3 为 latest） | `egui 0.37.0`（2026-10-15） |
| 发布节奏 | 2026-09-03 发布 0.1.0 起，7 周内 12 个版本，非常快 | 0.x 稳定迭代多年，每次 minor 有破坏性变更（官方明说） |
| 下载量 | 132,806（全部来自最近 2 个月） | 27,088,367（recent 6.9M） |
| 许可证 | Apache-2.0 | MIT OR Apache-2.0 |
| 底层 | GPUI（Zed），GPU 渲染 | epaint/wgpu 或 glow，immediate mode |
| 生产使用 | Longbridge Pro（券商桌面端） | Rerun Viewer、大量工具/游戏内嵌 |
| 文档 | gpui-kit.com、docs.rs、story/examples | egui.rs、docs.rs、demo、5 年社区积累 |

## 3. 能力对照（对 dshg 有意义的维度）

| 维度 | GPUI Kit | egui | 对 dshg 的影响 |
| --- | --- | --- | --- |
| 编程模型 | 保留式 + 实体/状态，渲染按需 | **立即模式**：每帧重跑 UI 代码，无回调 | 长会话 + 流式更新下，GPUI 的增量重绘更省；egui 需谨慎做局部重绘/虚拟化 |
| Markdown 渲染 | **内置**（含 HTML 混排） | 无，需 `egui_commonmark` 等社区 crate | dshg 的核心（消息流）→ GPUI 优势明显 |
| 代码高亮/编辑器 | **内置**编辑器 + Tree-sitter + LSP（200K 行） | 基础 `TextEdit`；高亮/编辑器靠社区 | 工具结果、diff、代码块 → GPUI 优势 |
| 虚拟化列表/表格 | 内置 VirtualList / Table / DataTable | `ScrollArea::show_rows` + `egui_extras` TableBuilder 可用 | 1 万事件会话两者都能做，GPUI 更省事 |
| Dock / 面板布局 | **内置** DockArea（拖拽、缩放、可序列化） | 社区 `egui_dock` | 侧栏/多面板 → GPUI 少一个依赖 |
| 主题 | 语义 token + 多主题 | Style 可改，但官方目标是"不做原生外观" | dshg 需要品牌化 → GPUI 更接近产品级 |
| 可访问性 | AccessKit 内建 | AccessKit 可选（macOS/Windows），Web 端实验 | 两者都可用；GPUI 更"默认就对" |
| 无头 UI 测试 | `gpui_kit::test` / TestWindowExt | `egui_kittest`（含 snapshot 测试） | 两者都能做回归；egui 的 snapshot 更成熟 |
| 原生菜单/托盘 | GPUI 侧有 macOS 菜单能力（Zed 实践） | 无内置原生菜单栏 | dshg 要 macOS 菜单 → GPUI 更顺 |
| Web/WASM | 站点声称支持 wasm32；但早期 README 对照表标 Web = No，**需实测** | **一等公民**（web demo、eframe） | 若未来做 Web 版，egui 明显更稳 |
| 跨平台 | macOS/Windows/Linux（站点声明；Linux 的 IME/Wayland 需实测） | macOS/Windows/Linux/Android/Web，最成熟 | Linux 用户多时 egui 风险更低 |
| 异步集成 | GPUI 自带 executor，`cx.spawn` 与 UI 生命周期结合 | 立即模式：UI 线程不能 `await`，要用 channel/watch | dshg 协议层是 async 密集型 → GPUI 的模型更贴合 |
| 二进制体积 | GPUI Kit README 对比表自报约 12MB（厂商口径） | 同表自报约 5MB | 差距可接受（dshg 不是以体积为卖点） |
| 生态/资料 | 新，但集中；示例与文档在快速补齐 | 极大：教程、第三方 crate、问答多 | 踩坑时 egui 更容易搜到答案 |
| 稳定性风险 | 版本迭代极快（0.1→0.8 七周），API 可能变 | 0.x 每次升级都可能破坏，但节奏可预期 | 两者都要锁版本；GPUI Kit 的"新"是主要风险 |

## 4. 对 dshg 的加权判断

按 dshg 的真实需求排序（消息渲染 > 长会话性能 > 审批交互 > 设置/插件 > 跨平台）：

| 需求 | 权重 | GPUI Kit | egui | 胜者 |
| --- | --- | --- | --- | --- |
| Markdown / 代码 / diff 渲染 | 高 | 内置 | 拼装 + 社区质量不一 | GPUI Kit |
| 1 万事件滚动与流式更新 | 高 | 虚拟化 + 按需渲染 | 需精细虚拟化，官方承认大滚动区可能慢 | GPUI Kit |
| 输入框（IME、中文、@ / 命令） | 高 | 原生 IME，Zed 级编辑器 | IME 可用；中文需自带字体 | 平手（GPUI 略优） |
| 审批/弹层/通知 | 中 | 组件齐全 | 需自己搭 | GPUI Kit |
| 面板布局/设置页 | 中 | Dock + 表单 | egui_dock + 手写 | GPUI Kit |
| 测试与回归 | 中 | 内建 harness | kittest + snapshot 更成熟 | egui |
| 跨平台/Web 未来 | 低（首版 macOS） | 声明支持，WASM 待验证 | 最成熟 | egui |
| 生态与排错 | 中 | 新、集中 | 大、成熟 | egui |
| 升级稳定性 | 中 | 变化快 | 变化可预期 | egui |

**结论**：在"首版 macOS + 重 Markdown/长会话 + 原生体验"的约束下，GPUI Kit 的现成能力直接省掉
Markdown、编辑器、Dock、虚拟化四块自研成本；egui 的优势主要在跨平台/Web、生态与测试成熟度，
而这些不是 v1 的胜负手。

## 5. 如果换 egui，成本与做法

- 需要自建/拼装：Markdown 渲染（egui_commonmark）、代码高亮与编辑器（egui_code_editor 等）、
  Dock（egui_dock）、虚拟化列表（show_rows）、通知/弹层、主题系统。
- 架构要从"实体 + 按需渲染"改为"每帧重建 UI + 外部状态"，流式事件要先落到共享状态再在帧内读取。
- 好处：Linux/Web 更稳、`egui_kittest` 的 snapshot 测试现成、社区资料多、体积更小。
- 粗估：UI 壳层多 2–4 周，且 Markdown/编辑器质量需额外打磨。

## 6. 何时重新评估（触发条件）

1. 协议实测后，若 GPUI Kit 在 macOS 上出现无法绕过的输入法/渲染/性能问题；
2. 产品决策变为"首版就要 Linux 或 Web"；
3. gpui-kit 升级破坏性变更频繁到每月都要改大量代码；
4. 组件缺失（如 diff 视图、文件树）导致自研量超过 3 周。

以上任一触发，就按第 5 节做 egui 可行性验证，再决定是否切换。

## 7. 备注

- 版本号以 crates.io 为准：`gpui-kit 0.8.0` 比站点文档（0.7.3）新，开工时锁版本并读对应 docs.rs。
- 二进制体积、跨平台/WASM 支持等数字来自厂商对比表或站点声明，**未实测**，标注为待验证。
- 本文只做选型判断，不改变 SPEC 的既有架构；切换框架属于 SPEC 级变更。
