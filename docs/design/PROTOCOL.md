# dshg 协议调研：dsh 0.2.0-rc.2 Client API

> 只读源码审阅产出，未运行 dsh。`$P` = `/Users/daniel/Library/Application Support/dsh-tauri/dependencies/dsh/node_modules/@deepseek-ai`，所有 `文件:行号` 相对 `$P`。
> **[确认]** = 源码/类型直接读到；**[推测]** = 由类型+文档推断，需实测。前提：dshg 只 attach 本机已运行的 `dsh --profile web`，走官方 Web 前端同一套 Client API。

## 1. 传输与握手

### 1.1 端点与浏览器信任栅栏

| 项 | 值 / 规则 | 依据 |
|---|---|---|
| HTTP 基址 | `http://127.0.0.1:<port>`（`host` 仅 `127.0.0.1`/`0.0.0.0`，`--port 0` 由 OS 分配） | `dsh-host-webserver/README.md`；`dsh-web-app/lib/startup.js:22` |
| unary RPC | `POST /api/<namespace>/<method>` | `dsh-client-connection/lib/types/api-path.d.ts:6`；`lib/index.js:676` |
| stream RPC | `WS /api/remote.mux`（**唯一** socket，多路复用） | `dsh-api-gateway/lib/types/stream-protocol.d.ts:4`；`lib/index.js:632-642` |
| 精确 Fetch 路由 | `GET/HEAD /api/file`、`POST /api/session/uploadFileBinary`、`GET/HEAD /api/session.export`、`GET /api/changes.summary`、`GET /api/present.host` | `dsh-api-session-controller/lib/types/media-references.js:70`；`dsh-client-file-upload/lib/types/protocol.d.ts:2`；`dsh-session-log-export/lib/types/routes.d.ts:7`；`dsh-client-ui-deliverables/lib/types/*.d.ts` |
| Host 头栅栏 | loopback（`localhost`/`[::1]`/`127.0.0.0/8`）或命中 `trustedHosts`（`host:port` 精确，portless `host` 匹配任意端口） | `dsh-client-connection/lib/index.js:121-131,192-210` |
| Origin / Sec-Fetch / 失败码 | Origin 存在时必须等于 Host；`sec-fetch-site: cross-site` 拒绝；栅栏失败 403，通过栅栏但无凭据 401 | 同上 `:212-216,587` |
| `--trusted-host` | 可重复追加栅栏白名单；`--host 0.0.0.0` 需 `DSH_PKG_ALLOW_LAN=1` | `dsh-web-app/lib/startup.js:22,40-41,47` |

认证为**进程级 launch token → 签名 cookie** 两段式（`dsh-client-connection/lib/index.js:244-250,374-410`）：① 进程启动生成随机 base64url token（仅内存），打印 `dsh web: http://127.0.0.1:<port>/?token=<token>`（`dsh-web-app/lib/index.js:203`）；② `GET /?token=<t>`（必须恰好一个 token、pathname 为 `/`）→ `303 Location: ./` + `Set-Cookie`；③ cookie 名 = `PREFIX + base64url(sha256(authority))`，值为 HMAC 签名的 `{version,authority,issuedAt,expiresAt}`，属性 `Path=/; HttpOnly; SameSite=Strict`、**无 `Secure`**、默认 30 天；④ 此后所有 `/api/*` 与 WS upgrade 只认 cookie——**不存在** query token（根路径外）、`Authorization` 头或 loopback 免认证档位；⑤ 非 index 静态资源公开。

### 1.2 generation / reconnect

| 概念 | 语义 | 依据 |
|---|---|---|
| generation 唯一来源 | 内部逻辑流 `$events`（`/api/remote.mux` 上 `endpoint:"$events"`，payload `{args:{}}`），与是否有监听无关 | `stream-protocol.d.ts:6,10-12`；`dsh-client-connection/README.md` |
| ready 帧 | Host 先挂增量监听，再发 `{type:'ready', clientId, host:{home}}`；收到才发布 generation | `stream-protocol.d.ts:27-32` |
| 超时 | 慢握手 3000ms 告警，15000ms 硬超时并取消该代 | `dsh-client-connection/lib/types/recovery-config.d.ts:12-18` |
| 退避 | 抖动 50%–100%，cap 500/1000/2000/4000/8000/10000ms 后持续；每次重试先要求 mux 关旧 socket 再开新 | 同上 `:5-11`；`RemoteStreamMuxClient.reconnect()` |
| 状态机 / 手动重连 | `'connected'\|'disconnected'\|'connecting'`（同值去重），`offline` 挂起重试、`online` 重置序列；`ctx.connection.reconnect()` 中断当前代、清零 attempt、立即重试 1 | `dsh-client-connection/lib/types/client/connection.d.ts:17,60` |
| 换代广播 | 每建立一代 `emit('connection/reset')`，契约写明"wire-derived caches 必须重拉" | `dsh-api-gateway/lib/client.js:1608`；`dsh-client-connection/lib/types/client/index.d.ts:11-14` |

**[确认]** generation 是"事件流代次"，不是 HTTP 会话；unary 请求不携带 generation id。

## 2. 请求通道

### 2.1 unary envelope（HTTP）

```jsonc
// 请求：POST /api/<ns>/<method>，content-type 强制 application/json（否则 415）
{ "type": "client-request", "rpcId": "<uuid>", "method": "<ns>/<method>", "payload": { "args": { /* 具名参数 */ } } }
{ "type": "server-response", "rpcId": "<同一个>", "result": { "ok": true, "value": <业务值> } }          // 成功
{ "type": "server-response", "rpcId": "<同一个>", "result": { "ok": false, "error": {"code","message","details"} } } // 业务/分发失败，HTTP 仍 200
```

| 规则 | 依据 |
|---|---|
| 类型定义 `ClientRequest`/`ServerResponse`/`ConnectionRpcFailure{code,message,details}` | `dsh-client-connection/lib/types/rpc.d.ts:62-73,13-25` |
| `method` 必须等于路径里的 endpoint，否则回 `gateway/bad-request` 错误 envelope；响应 `rpcId` 必须与请求一致（客户端本地校验，不符即抛） | `dsh-client-connection/lib/index.js:678-693`；`lib/client.js` `createWebConnectionRpc` |
| endpoint 段字符集 `[A-Za-z0-9_$.-]+`；channel `/[A-Za-z0-9._~-]+`（客户端本地拒绝非法目标） | 同上 `assertTarget` |
| Typert 方法 `payload` 固定 `{args:{<形参名>:值}}`；`signal` **不**上线（server 侧注入） | `dsh-api-gateway/lib/client.js:1792,1815,1825-1848`；`dsh-typert-protocol/lib/types/types.d.ts:222-226` |
| 调用方取消 → 折叠为 `gateway/cancelled`；传输故障 → `gateway/internal`；**从不 reject**，只在 `RemoteResult` 错误分支 | `dsh-api-gateway/README.md`（Client service 段）；`dsh-typert-protocol/lib/types/types.d.ts:41-50` |
| 二进制 unary 结果：响应 `multipart/form-data`，part `metadata` 放 envelope（字节字段为 `null` 占位）+ `attachments:[{codec:"bytes",part,path}]`，其余 part 是字节 | `dsh-client-connection/README.md`；`lib/client.js` `parseBinaryResponse` |

### 2.2 stream（WebSocket）

所有逻辑流共用一个 socket，**只接受文本帧**，非字符串帧视为协议错误并 `close(4002)`（`dsh-api-gateway/lib/types/client/stream-client.js`）。

| 项 | 值 | 依据 |
|---|---|---|
| C→H 帧 | `{type:"open", streamId, endpoint, payload}` / `{type:"item", streamId, value?}` / `{type:"end", streamId}` / `{type:"cancel", streamId}` | `stream-protocol.d.ts:126-141` |
| H→C 帧 | `{type:"item", streamId, value?}` / `{type:"error", streamId, error:{code,message,details}}` / `{type:"end", streamId}` | `stream-protocol.d.ts:149-160` |
| `streamId` | 客户端生成 UUID；`item` 缺 `value` 表示顶层 `undefined`；`end` 为 uplink 半关闭 | `stream-client.js`；`stream-protocol.d.ts:131-138` |
| 心跳 | Host 每 `websocketHeartbeatIntervalMs`（默认 2000ms）发 Ping，上轮未回 Pong 即终止 | `dsh-api-gateway/README.md`；`lib/index.js:597` |
| uplink 缓冲 | `streamInboxBytes` 默认 262144；溢出 `gateway/uplink-overflow`，`end` 后再发 `gateway/protocol` | `dsh-api-gateway/README.md` |
| 重复 `open` 同 streamId | 唯一会关 socket 的客户端错误（其余按 streamId 丢弃） | 同上 |
| 重连 | `RemoteStream` 在 carrier 换代后**重开**逻辑流，不重放 item；恢复语义由各域 cursor/baseline 自负 | `dsh-api-gateway/lib/types/client/remote-stream.js` |

## 3. 方法清单

端点全集来自各包 `lib/typert.remote-client.d.ts` 生成的 `TypertRemoteMap`（权威且完整）；参数名即 `args` 的键，`signal?` 为可选取消。

### 3.1 session / 文件引用 / skill（`dsh-api-session-controller/lib/typert.remote-client.d.ts`）

| 方法 | 参数 → 返回 |
|---|---|
| `session/list` | `_request:SessionListRequest{cursor?}` → `SessionListValue{items:SessionSummary[]}` |
| `session/page` | `request:SessionPageRequest{address,throughSeq,beforeSeq?,maxMessages?,turnWindow?}` → `SessionPage{records,hasMore}` |
| `session/follow` | `request:SessionFollowRequest{address,assistantStream?,…}` → **stream** `SessionFollowFrame` |
| `session/control` | — → **stream** `SessionControlFrame`（全 Host projection 基线+增量） |
| `session/projections` | `{sessionId}` → `SessionProjectionsValue\|null` |
| `session/create` | `{workspaceId?,cwd?,sessionId?,agentPreset?}` → `{sessionId,agentPreset?}` |
| `session/fork` / `session/rename` | `{sessionId,atSeq?}` → `{sessionId}` / `{sessionId,title}` → `{title,seq}` |
| `session/selectModel` | `{sessionId,provider,model,reasoningEffort?}` → `{selected}` |
| `session/prompt` | `{sessionId,requestId,mode:'queue'\|'steer',content:PromptContentPart[],clientTimeZone?}` → `{accepted:true}` |
| `session/cancel` / `session/updateQueue` | `{sessionId}` / `{sessionId,itemId,action:QueueAction}` → `{accepted:true}` |
| `session/attachment` | `{sessionId,attachmentId}` → `{attachment,data}`（base64） |
| `session/search` | `{query}` → `{items,hasMore}` |
| `session/modelCatalog` / `initializeDefaultModel` | — → `ModelCatalog` / `void` |
| `session/canOpenWorkspacePath` / `workspacePathApplications` | — → `boolean` / `{path}` → `SessionWorkspacePathApplication[]` |
| `session/openWorkspacePath` | `{path,action?:'reveal',application?}` → `{opened:true}` |
| `fileReferences/list` | `agentId:SessionId, query:string` → `FileReferenceCandidate[]` |
| `skills/list` | `{sessionId}` → `{skills:SkillEntry[]}` |

### 3.2 settings / credentials（`dsh-api-settings-controller/lib/typert.remote-client.d.ts`）

| 方法 | 参数 → 返回 |
|---|---|
| `settings/describe` | — → `SettingsDescribeValue{writable,hasDocument,namespaces:SettingsNamespaceView[]}` |
| `settings/update` \| `replace` \| `mutate` | `ns, patch\|section\|ops, expectedRevision` → `SettingsNamespaceView` |
| `settings/openSettingsDocument` | — → `{opened:true}` |
| `credentials/describe` \| `set` \| `unset` | `refs[]` / `ref,value` / `ref` → `Record<string,CredentialInfo>` / `void` |

写冲突 → `settings/conflict{ns,expected,actual}`；其他拒绝 → `settings/rejected{ns}`、`credential/rejected{ref}`（`dsh-api-settings-controller/lib/types/types.d.ts:9-33`）。

### 3.3 workspace / files / directoryPicker

| 方法 | 参数 → 返回 |
|---|---|
| `workspace/create` \| `rename` \| `delete` | `{path}` \| `{workspaceId,title}` \| `{workspaceId}` → `{workspace,created}` \| `{workspace}` \| `{deleted:true}` |
| `workspace/insertBefore` \| `insertSessionBefore` | `{workspaceId,beforeWorkspaceId?}` \| `{workspaceId,sessionId,beforeSessionId?}` → `{workspaceIds}` \| `{workspace}` |
| `workspace/pinSession` \| `unpinSession` | `{sessionId}` → `{pinnedSessionIds}` |
| `workspace/archiveSession` \| `unarchiveSession` | `{sessionId,stopActivity?}` \| `{sessionId}` → `{archivedSessionIds}` |
| `workspace/initializeDefault` / `workspace/follow` | — → `WorkspaceValue\|undefined` / **stream** `WorkspaceFollowFrame` |
| `directoryPicker/list` \| `pick` \| `createDirectory` | `path:string\|undefined` \| — \| `path,name` → `DirectoryListing` \| `string\|null` \| `string` |
| `workspaceFiles/list` / `read` | `workspaceFileScopeId:SessionId, path` → `WorkspaceDirectoryListing` / `…, path, range{offset?,limit?}` → `WorkspaceFileText` |
| `workspaceFiles/readBytes` | `…, path, options{range?,baseFile?}` → `WorkspaceFileBytes`（字节，走 multipart） |
| `workspaceFiles/stat` \| `changes` | `…, path` → `WorkspaceFileStat` / **stream** `{kind:'ready'\|'change'}` |

路径词汇：`read/readBytes/stat/changes` 收**绝对路径**，`list` 收**workspace 相对路径**（`dsh-api-workspace-files/lib/types/types.d.ts:5-14`）。以上依据同目录 `typert.remote-client.d.ts`。

### 3.4 job / terminal / account

| 方法 | 参数 → 返回 | 依据 |
|---|---|---|
| `job/list` | `{sessionId}` → **stream** `{type:'rows',jobs}`（整集替换，首帧即真值） | `dsh-api-job-controller/lib/types/types.d.ts:14-24` |
| `job/follow` | `{sessionId?,jobId,from?}` → **stream** `opened/output/status`（按字节 offset 续读） | 同上 `:56-85` |
| `job/kill` | `{sessionId,jobId}` → `{outcome:'requested'\|'already-finished'}` | 同上 `:28-37` |
| `terminal/list` \| `create` \| `close` \| `rename` | `sessionId` \| `agentId,{shellPath?,id,cols,rows}` \| `agentId,id` \| `agentId,id,title` | `dsh-api-terminal-controller/lib/typert.remote-client.d.ts` |
| `terminal/shells` \| `environment` \| `retain` | `agentId` \| `agentId` \| `sessionId,id`（**stream** `{type:'retained'}`） | 同上 |
| `terminal/follow` | `agentId,id,attachmentId` → **stream** `{snapshot\|output\|state}` | 同上 |
| `terminal/write` \| `resize` | `agentId,id,attachmentId,data` \| `…,cols,rows` → `void` | 同上 |
| `account/getState` \| `watch` | `(client:AccountClientMetadata)` / — → `AccountView` / **stream** `AccountView` | `dsh-api-account-controller/lib/typert.remote-client.d.ts` |
| `account/getProfile` \| `getBalance` \| `watchExpiry` | `client` → `Profile\|null` / `Balance\|null` / — → **stream** `'session-expired'` | 同上 |
| `account/startSignIn` \| `cancelSignIn` \| `signOut` | `client,callbackOrigin,loginSource` \| `attemptId` \| `client` → `AccountView` | 同上 |

### 3.5 其余可写命名空间

| 命名空间 | 端点 | 依据包 |
|---|---|---|
| `subagents` | `prompt(request)`、`interruptByParent(childSessionId,parentSessionId,'continuable')` | `dsh-subagent` |
| `goals` | `create/get/edit/pause/resume/complete/clear`（首参均 `agentId:SessionId`） | `dsh-goal` |
| `commands` | `list(agentId)`、`execute(agentId,line,submittedAttachments)` | `dsh-commands` |
| `userQuestions` | `answer(agentId,callId,answer)`、`attachWait(agentId,callId)` **stream** `{remainingMs}` | `dsh-user-questions` |
| `schedule` / `llm` | `catalog/list/update/delete/history` / `listProviders/listConfigurableProviders/discoverModels` | `dsh-schedule`、`dsh-llm` |
| `pluginInventory` / `pluginManager` / `permissionPresets` | 见 §7 / `catalog()` | `dsh-host-plugin-inventory`、`dsh-plugin-manager`、`dsh-permission-presets` |
| `agentPresets` / `fileUploads` / `messageFeedback` | `list/read/select`、`upload(agentId,request)`、`list/put/delete` | 三包 |
| `sessionFeedback` / `sessionReferenceResolver` / `productAnalytics` / `speech` / `officeToPdf` / `dynamicCordisRunner` | `record`、`candidates`、`enabled/report/watchPolicy`、`catalog/configure/prepare/transcribe/cancelPreparation/follow`、`render/generation`、12 个 `*` | 六包（见各包 `typert.remote-client.d.ts`） |

## 4. 事件 / 通知

**[确认] 重要纠正**：Web Client API **没有** `session.event` / `session.status` / `subagent.started` 这类事件名——那是 `dsh-sdk-protocol`（stdio，见 §8）的命名。Web 侧分两层：

1. **转发型 Host 事件**（`/api/remote.mux` 的 `$events` 流）。允许列表是唯一真源：`dsh-api-remotes/lib/types/remote-events.d.ts:11-70`，共 27 项：
   - `emit`（单向、换代不重放）：`agent-preset/selected`、`api-session/{added,removed,status,activity,error}`、`commands/change`、`deepseek-account/{session-expired,model-sign-in-required}`、`credentials/{record-updated,reference-updated}`、`goal/activation-changed`、`cordis/{request-run,request-run-resolved,dynamic-package,dynamic-retract,inspect-query,inspect-query-resolved}`、`llm/adapters-updated`、`permission-presets/catalog-changed`、`plugin-manager/{changed,install-log,install-state}`、`settings/document-updated`、`schedule/changed`
   - `waterfall`（Agent 作用域，需应答）：`approval/request`、`user-questions/request`
2. **域内流帧**：会话里发生的一切（user/assistant/tool/turn 事件 + assistant 增量）走 `session/follow` 的 `SessionFollowFrame`；列表级 running 走 `api-session/status(sessionId,running)`；**subagent 生命周期不转发到 Web**，用 `SessionAddress{kind:'subagent',parentSessionId,childSessionId,mode}` 单独 `session/follow`（`dsh-api-session-controller/lib/types/types.d.ts:386-394`）。

| 帧 | 形状 | 依据 |
|---|---|---|
| emit | `{type:'emit', event, args: unknown[]}` | `stream-protocol.d.ts:36-40` |
| waterfall / cancel | `{type:'waterfall', event, eventId, agentId, request}` / `{type:'cancel', eventId}` | 同上 `:42-53` |
| 应答 | `POST /api/$events/result`，payload `{args:{clientId,eventId,outcome}}` | 同上 `:69-81`；`lib/client.js:855-870` |

### 断线重连后如何补拉

| 数据 | 恢复机制 |
|---|---|
| 全局缓存（settings 镜像、skill 目录、账号视图…） | 监听本地 `connection/reset` 全量重拉 |
| 会话事件窗口 | `session/follow` 重开必发 `{type:'snapshot', header, cursor, records, hasMore, projections, assistantStream?}`；`RemoteJournalStream` 用 `session/page`（`throughSeq`/`beforeSeq`）修 gap，倒序/半重叠即失败（`types.d.ts:511-524`；`client/transport.d.ts:31-40`） |
| 会话 projection（list 元数据、model 选择） | `session/control` 每代先 `{type:'baseline',value:{projections}}`，随后 `{type:'projection'}`（`types.d.ts:526-545`） |
| Workspace / Job / 终端 / 文件监听 | 各自基线：`workspace/follow` baseline、`job/list` 整集、`terminal/follow` 首帧 `snapshot`、`changes` 首帧 `{kind:'ready'}` |
| 纯 unary 列表（`session/list`、`pluginInventory/list`、`settings/describe`） | 无推送，只能由 `connection/reset` + 对应 `emit` 事件触发重拉 **[推测]**（reset 语义有源码，各包重拉逻辑在各自 client 插件内） |

## 5. 关键域模型

| 模型 | 关键字段 | 所在包 / 文件 |
|---|---|---|
| `SessionWireHeader` / `SessionSummary` | header `{version,id,createdAt,cwd?,parentSession?,isSeeded,origin?:'subagent',delegationDepth?,agentPreset?}`（`version` 是**会话日志格式版本**，非协议版本）；列表行 `{sessionId,updatedAt,running,blank,agentAvailable,parentSessionId?,origin?,cwd?,projections?:{kind:'cached'\|'sequenced',asOfSeq,values}}`，**不含 title** | `dsh-api-session-controller/lib/types/types.d.ts:407-418,159-170` |
| 标题 | durable 事件 `session/title`（latest-wins）；list 侧只有 `sessionListMetadata{blank,lastPromptAt}` | `dsh-session-title/lib/types/index.d.ts:37-42`；`types.d.ts:38-45` |
| `SessionEvent` / `SessionWireEvent` | 日志态 `{type,seq:SessionSeq,time,data,ignorable?,sourceEventSeqs?,surfaceOp?}`；传输态同名但 `data:JsonValue`，`surfaceOp` 仅出现在 surface 事件。`type` 全集：`turn/start turn/end step/start step/end user/message developer/message system/message assistant/message assistant/attempt tool/call tool/result request/header request/context session/end-seed`，可扩展（`model/selection`、`session/title`、`approval/asked`、`approval/decided`、`hook/*`） | `dsh-session/lib/types/types.d.ts:294-395,489-510`；`dsh-api-session-controller/lib/types/types.d.ts:432-441` |
| `ContentBlock` / `ToolCall` | `ContentBlock` 判别式 union（merge-extensible）：`text`/`reasoning`/`image`(durable ref)/`tool-call{id,name,arguments:raw JSON string}`/`tool-addition`；`ToolCall` 事件 `tool/call{turn,step,callId,name,arguments}`，结果 `tool/result{turn,step,message:ToolResultMessage,error?:{name,code,reason?},meta?:JsonValue}`，`meta` 为工具私有展示负载（如 fs diff） | `dsh-llm/lib/types/types.d.ts:46-126`；`dsh-session/lib/types/types.d.ts:354-388` |
| Assistant 增量（client-only，不入日志） | `SessionAssistantStreamFrame = start/chunk/end`；基线 `{revision, activeAttempt{attemptId,startedAfterSeq,turn,step,nextIndex,stream}}` | `dsh-api-session-controller/lib/types/types.d.ts:465-509` |
| `Approval` | 审计事件 `approval/asked{id,toolName,callId?,reason?}` / `approval/decided{id,outcome}`；`ApprovalOutcome='allowed-once'\|'rejected'\|'cancelled'\|'unavailable'` | `dsh-user-approval/lib/types/types.d.ts:20-58` |
| `Workspace` / 文件 / `Settings` | `WorkspaceView{workspaceId,path,title,sessionIds,createdAt,updatedAt}`；`WorkspaceFileStat{absolutePath,version,bytes?}`；`WorkspaceDirectoryListing{path,entries,truncated}`；`SettingsNamespaceView{ns,schema(序列化 schemastery),value,base?,user?,applies:'live',secrets[{path,set}],revision}`，`SettingsPathOpView={op:'set',path,value}\|{op:'unset',path}` | `dsh-api-workspace-controller/…/types.d.ts:20-30`；`dsh-api-workspace-files/…/types.d.ts:30-40,100-120`；`dsh-settings/lib/types/types.d.ts:17-54` |
| 插件清单 / Job | `PluginInventorySnapshot{managementAvailable?,entries,agentPresets?}`（entry 含 `condition`/`fiberPhase`）；`JobView`+`JobChunk`（`dsh-jobs/view`） | `dsh-host-plugin-inventory/lib/types/types.d.ts:52-63`；`dsh-api-job-controller/lib/types/types.d.ts:56-85` |
| 错误码 | 通用 `gateway/bad-request`/`cancelled`/`internal`；Gateway 另 19 个 `gateway/*`；各域自注册 `session/not-found`、`workspace/*`、`terminal/*`、`workspace-file/*`、`settings/*`、`job/not-found` | `dsh-typert-protocol/lib/types/types.d.ts:41-50`；`dsh-api-gateway/lib/types/remote-error-codes.d.ts` |

## 6. 审批流

| 环节 | 事实 | 依据 |
|---|---|---|
| 到达形式 | `$events` 上的 **waterfall 帧**，非 RPC 拉取：`{type:'waterfall', event:'approval/request', eventId, agentId, request:{toolName,callId?,reason?,displayReason?{en,…}}}`；`agent` 与 `signal` 在传输前被剥掉，`agentId` 单独放帧上 | `dsh-api-remotes/lib/types/remote-events.d.ts:16`；`dsh-user-approval/lib/types/types.d.ts:60-78`；`stream-protocol.d.ts:42-48,57-60` |
| 作用域 | Agent 作用域 waterfall；客户端用 `agentId` 解析对应 Context，解析不到必须回 `{kind:'next'}` 委派给下一个应答者 | `dsh-api-gateway/README.md`；`lib/client.js:820-860` |
| 回应 RPC | `POST /api/$events/result`，`outcome` = `{kind:'result',value:ApprovalOutcome}` \| `{kind:'rejected',error}` \| `{kind:'next'}` | `stream-protocol.d.ts:69-81` |
| 可选答案 | 官方 UI 只实现 **allow-once** 与 **reject**；**没有** `always-allow` 的一次决策 | `dsh-client-ui-approval/README.md` "Known Limitations" |
| `always-allow` 的正规路径 | 持久权限策略由 Host 侧 permission-presets 拥有；Web 只读 `permissionPresets/catalog` + 收 `permission-presets/catalog-changed` | 同上；`dsh-permission-presets/lib/typert.remote-client.d.ts` |
| 审计 | `approval/asked`/`approval/decided` 是 **log-only** 会话事件（无 `surfaceOp`），随 session 日志到达 | `dsh-user-approval/lib/types/types.d.ts:35-58` |
| 撤销 | 收到 `{type:'cancel', eventId}` 或流代次结束必须停止等待并让处理函数 settle | `stream-protocol.d.ts:49-53`；`dsh-api-gateway/README.md` |

`user-questions/request` 是同一 waterfall 机制的第二例（`remote-events.d.ts:69-70`），应答走 `userQuestions/answer` RPC。

## 7. 插件启停

| 能力 | 端点 | 返回 / 说明 |
|---|---|---|
| 清点（只读快照） | `pluginInventory/list()` | `PluginInventorySnapshot`：entry、`fiberPhase`、`condition`、可选 agentPresets 分组 |
| 列出可管理插件 / bundle | `pluginManager/listPlugins()`、`listBundles()` | `PluginInfo[]`（含"为何不可改"）、`BundleInfo[]` |
| **启用/停用单个插件** | `pluginManager/setPluginEnabled(id: PluginEntryId, enabled)` | `ChangeResult` |
| **启用/停用 bundle 层** | `pluginManager/setBundleEnabled(name, enabled)` | `ChangeResult` |
| 安装 / 移除 / 预检 / 等待 / 取消 | `installBundle(spec,options?)`、`removeBundle(name)`、`inspect(spec)`、`waitForInstall(requestId)`、`cancelInstall(requestId)` | `ChangeResult` / `PluginSpecInspection` |
| registry / 版本豁免 | `registries()`、`listVersionExemptions()`、`setVersionExemption(pkgVersion,runtimeVersion,enabled,acceptRisk?)` | — |

**是否热生效**：`ChangeResult.application ∈ {'applied','restart-required','overridden','failed','cancelled'}`（`dsh-plugin-manager/lib/types/types.d.ts:93-118`）。`setPluginEnabled` 契约是"持久化期望值并**在 live profile 上应用**"（`lib/types/index.d.ts:84-90`），`setVersionExemption` 明确"Startup-only profiles require restart"（`:66-72`）。

**[确认]** 结论：live profile 下启停**可以**热生效，但**不可假设**——必须读 `ChangeResult.application`；`restart-required` 时提示重启 dsh，`overridden` 表示被更高优先级配置（如 `cordis.patch.yml`）覆盖，UI 要显式展示。插件**配置**走 §3.2 的 `settings/*`，不与 pluginManager 混用。

## 8. stdio 方案对比（不采用，仅参照）

`dsh-sdk-protocol` 是完全独立的通道：**换行分隔 JSON-RPC 2.0**，方法集只有 3 请求 + 4 通知（`initialize`/`session/prompt`/`shutdown`；`session.event`/`session.status`/`subagent.started`/`subagent.finished`），服务端为 `dsh-sdk-jsonrpc-server`（`dsh-sdk-protocol/README.md` method table）。

| 维度 | Web Client API（dshg 采用） | SDK stdio |
|---|---|---|
| 传输 | HTTP POST unary + 单条多路复用 WS | 进程 stdin/stdout 行协议 |
| 方法面 | ~90 端点，覆盖 session/workspace/files/jobs/terminal/plugins/settings/account | 3 个请求 |
| 事件 | 27 个转发事件 + 每域可续传 stream | 4 个全局通知（`session.event` 全量未过滤） |
| 重连 | generation + 退避 + baseline 恢复 | 无（进程生命周期即会话） |
| 版本协商 | 无 | 无（README 明示 `serverInfo.version` 不校验，pre-release 无兼容承诺） |

**[确认]** 二者共享会话词汇（`SessionEvent`、`ContentBlock`、`SubagentStopReason`），但**方法名不通用**：dshg 若将来加 stdio 兜底，需要独立适配层。

## 9. 兼容与风险

| # | 风险 | 现状 / 依据 | 应对 |
|---|---|---|---|
| R1 | **无任何版本协商** | ready 握手只有 `{type,clientId,host:{home}}`，无 version/capability（`stream-protocol.d.ts:27-32`）；SDK 侧亦明确无版本协商 | 端点存在性探测；把 `gateway/method-unavailable` 当能力缺失降级，而非崩溃 |
| R2 | RC 期方法面会变 | 端点全部由代码生成（`typert.remote-client.d.ts` 头注释 "Generated … do not edit"），同版本 `0.2.0-rc.2`；增删无兼容层 | 端点表按包版本快照 + 严格白名单；未知 `code` 按字符串透传 |
| R3 | **token 与 cookie 的获取/绑定脆弱** | launch token 只在 dsh 进程内存 + stdout 那行（`dsh-web-app/lib/index.js:203`），**无任何查询 API**；cookie 名与签名绑定 `hostname[:port]`（`lib/index.js:284`）且无 `Secure` | 必须让用户粘贴 URL 或解析 `dsh --profile web` 输出；端口变化须重新换 token；拒绝非 loopback 的明文 cookie 传输 |
| R4 | 未命中路由回 404 而非错误信封 | 未认领 endpoint 直接 `404 "not found"`（`lib/index.js:620`） | 把 HTTP 404 与 `200+ok:false` 当两种失败语义 |
| R5 | 二进制仅支持 unary | README：binary parameters/events/streamed binary results 均不支持 | 文件字节走 `workspaceFiles/readBytes`（unary multipart），不做成流 |
| R6 | 重连不重放单向事件 | `emit` 帧换代后不补发（`dsh-api-gateway/README.md` Known Limitations） | 事件驱动的状态必须能从某条流 baseline 重建 |
| R7 | `session/follow` 的 surface 语义复杂 | `surfaceOp` 替换 + `sourceEventSeqs` 引用压缩结果；缺 `ignorable` 的未知事件必须**拒绝重建**而非跳过（`dsh-session/lib/types/types.d.ts:497-508`） | 渲染层要做 surface 折叠，不能简单 append；未知必需事件报错 |
| R8 | `$events` 是 Gateway 内部端点 | 名字带 `$`，文档称 "Gateway-internal"（`stream-protocol.d.ts:5-8`） | 视为准稳定，隔离在一层 transport adapter 后 |
| R9 | 流式上传绕过体积上限 | `requestBody:'streaming'` 的 `/api/session/uploadFileBinary` 不受 300MiB 聚合上限约束 | 大文件上传需真实 backpressure，不可整包缓冲 |

**dshg 接入必须实测的点**：① 真实 `GET /?token=…` 的 303+`Set-Cookie`（cookie 名/属性/有效期），以及 **cookie 是否足以完成 WS upgrade**（源码判据 `admit()` 共用，未实测）；② `session/follow` 的真实帧序列：重连后是否只发 `snapshot`、`cursor`/`records` 边界、`assistantStream` 何时出现，并配合 `session/page` 验证 gap 修复；③ 一个 `approval/request` waterfall 的端到端往返（`displayReason` 是否真在 wire 上、`signal` 是否确实不在、`outcome.kind` 实际取值）；④ `pluginManager/setPluginEnabled` 在用户真实 profile 上返回 `applied` 还是 `restart-required`；⑤ 对 §3 每个端点发一次空 `args` 探测，确认哪些真的挂着。

## 10. 一句话总结

**最不确定、最需尽早实测的 3 件事**：① launch token 的唯一来源是 dsh 的 stdout（没有查询 API），"attach 已运行实例"必须先解决 token/URL 怎么拿到；② `session/follow` 的 `snapshot + surfaceOp + assistantStream` 帧序列与重连 gap 修复的真实行为，直接决定会话渲染正确性；③ `pluginManager/setPluginEnabled` 到底 `applied` 还是 `restart-required`（连同 `$events` waterfall 审批往返的真实 payload），这两点决定"插件面板"和"审批 UI"能否做成无重启体验。
