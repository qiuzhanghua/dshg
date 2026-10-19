# dshg 与官方 Web UI 的功能对照（PARITY）

- 版本：v0.1 草案
- 日期：2026-10-20
- 依据：dsh 0.2.0-rc.2 的 `dsh-client-ui-*` 包清单 + [需求纲要](../requirements/OUTLINE.md) 的 MVP 边界
- 说明：本表是**范围对照**，不是像素级功能等价清单；官方 Web UI 迭代快，每个里程碑前需复核。

## 图例

- **MVP**：首版（v1, macOS）必须
- **后续**：v1 之后按路线图做
- **不做**：不在 dshg 的产品范围
- **待定**：需要评审

## 对照表

| 官方 Web UI 能力（包） | 内容 | dshg v1 |
| --- | --- | --- |
| `session` / `conversation` / `chat` | 会话列表、消息流、会话切换 | **MVP** |
| `renderer` / `primitives` / `tool` | 消息渲染、工具调用卡片 | **MVP**（工具卡片先用通用渲染 + 常见工具定制） |
| `approval` / `permission-presets` | 审批请求、权限预设 | **MVP** |
| `attachment` | 图片/文件附件 | **MVP**（图片粘贴；文件拖拽后续） |
| `reference` | `@` 文件引用 | **MVP** |
| `commands` / `input-trigger` | `/` 命令与触发菜单 | **MVP**（基础命令） |
| `model-selection` | 模型切换 | **MVP**（放在设置/顶栏） |
| `settings-*`（general/models/shell/web-search/agent-loop/subagent/account） | 各类设置 | **MVP**（模型、shell、沙箱、主题、语言；其余逐项补） |
| `plugin-manager` / `settings-plugins` / `settings-plugin-inventory` | 插件清单、启用/禁用 | **MVP**（启停） |
| `skill` | 技能列表与激活 | **后续**（先只读展示） |
| `jobs` | 后台任务面板 | **后续** |
| `plan` | 计划模式 | **后续** |
| `goal` | 目标管理 | **后续** |
| `schedule` | 定时任务 | **后续** |
| `subagent` | 子 agent 会话树 | **后续**（MVP 用工具卡片兜底） |
| `workflow-run` | workflow 运行视图 | **后续** |
| `deliverables` | 交付物列表 | **后续** |
| `trajectory` | 轨迹/事件视图 | **后续** |
| `settings-session-log` | 会话日志查看 | **后续** |
| `layout` / `sidebar*` / `slots` / `theme` | 布局、侧栏、主题 | **MVP**（会话 + 文件只读侧栏；无终端/浏览器/文档预览） |
| `sidebar-terminal` | 内置终端 | **不做**（v1 明确排除） |
| `sidebar-browser` | 内置浏览器 | **不做** |
| `sidebar-documentpreview` | 文档预览 | **后续** |
| `directory-picker-browse` / `native` | workspace 目录选择 | **MVP**（macOS 原生选择器） |
| `shortcuts` | 快捷键 | **MVP**（基础：新建会话、发送、切换会话、搜索） |
| `open-in-app` | 用外部应用打开 | **后续** |
| `message-feedback` | 消息反馈/点赞 | **不做**（v1） |
| `brand-official` | 官方品牌资源 | **不做**（dshg 自有品牌，暂缓） |
| `cordis` | 插件运行时相关 UI | **不做**（不重写插件运行时） |
| 语音输入等实验能力 | — | **不做**（v1） |

## dshg 的差异点（官方 Web UI 没有或形态不同）

1. **原生桌面集成**：菜单栏、Dock 通知、系统快捷键、拖拽、窗口恢复。
2. **引擎连接引导**：官方 Web UI 由启动命令自带地址；dshg 需要"探测 → 引导 → 重连"。
3. **多窗口**：[待定] v1 单窗口；分屏/多窗口留给后续。
4. **本地诊断**：连接状态、协议版本、日志导出。
5. **不做浏览器信任栅栏之外的鉴权**：只连 loopback，无远程模式。

## 复核清单（每个里程碑）

- 对照 dsh 当前版本的 `dsh-client-ui-*` 包，确认没有新增的"必须有"能力。
- 确认官方 Web UI 的关键交互（审批、停止、错误提示）没有被 dshg 做成不同语义。
- 更新本表的"后续/不做"分类，避免范围漂移。
