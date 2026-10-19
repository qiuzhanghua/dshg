# dshg 协议实测计划（VALIDATION）

- 版本：v0.1 草案
- 日期：2026-10-20
- 目的：把 [PROTOCOL.md](PROTOCOL.md) 里"源码确认但未运行验证"的结论逐条实测，
  为 M0 扫清阻塞项。**本阶段只验证协议，不写产品代码。**
- 依据：dsh `0.2.0-rc.2`，macOS，本机 loopback。

## 1. 前提与安全

- 需要真实运行 dsh（会执行工具）。**用临时目录**（如 `/tmp/dshg-validate`）做 workspace，
  审批测试里只跑 `echo` 之类的无害命令，不要指向真实项目。
- 不改 dsh 源码、不改全局配置；插件启停用例要记录初始状态并还原。
- 实测结果写进 `VALIDATION-RESULTS.md`（模板见第 6 节），不要直接改 PROTOCOL 的"确认"标记，
  除非已复现。

## 2. 环境准备

```sh
# 1) 启动（不要让 dsh 打开浏览器；--port 0 让 OS 分配，也可固定 3080）
dsh --profile web --no-open --port 0
# 终端会打印：dsh web: http://127.0.0.1:<port>/?token=<token>
# 记下整行 URL。

# 2) 变量
export BASE="http://127.0.0.1:<port>"
export TOKEN="<token>"                 # 从 URL 里取
export JAR="/tmp/dshg-cookies.txt"     # cookie jar
export WORK="/tmp/dshg-validate"       # 测试 workspace
mkdir -p "$WORK"
```

工具：

| 用途 | 工具 | 备注 |
| --- | --- | --- |
| HTTP | `curl`、`jq` | 系统自带 |
| WebSocket | `npx -y wscat` 或 `websocat` | 需要能带 `Cookie` 头；wscat 用 `-H` |
| 抓帧记录 | 终端日志重定向 | `wscat … 2>&1 \| tee /tmp/dshg-ws.log` |

`dsh` 命令来自 cot 环境：`c && dsh …`。

## 3. 通过判据

| 级别 | 含义 |
| --- | --- |
| **PASS** | 与 PROTOCOL 描述一致（允许无关字段差异），且后续用例可继续 |
| **PASS\*** | 行为一致但字段/时序有差异，记录差异并可继续 |
| **BLOCK** | 阻塞后续用例或推翻当前方案，必须回到文档修订 |
| **FAIL** | 与预期不符，记录实际响应，不擅自修改结论 |

每个用例至少记录：`时间 / 命令 / 实际响应（脱敏）/ 判定 / 备注`。

## 4. 用例

### V1 未认证访问与错误 token（BLOCK 级：认证是否存在）

```sh
# a) 无 cookie 访问根路径 → 401
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/"

# b) 错误 token → 401（并确认不会下发 cookie）
curl -s -i "$BASE/?token=wrong-token" | head -5

# c) 无 cookie 调 API → 401（确认栅栏/认证在路由之前）
curl -s -o /dev/null -w '%{http_code}\n' \
  -H 'Content-Type: application/json' \
  -d '{"type":"client-request","rpcId":"v1","method":"settings/describe","payload":{"args":{}}}' \
  "$BASE/api/settings/describe"
```

判据：a/b/c 均 **401**；b 的响应无 `Set-Cookie`。

### V2 token → cookie 交换（BLOCK 级：首连 UX 的基础）

```sh
curl -s -i -c "$JAR" "$BASE/?token=$TOKEN" | sed -n '1,20p'
awk '$6 ~ /^dsh-session-/{print $6"="$7}' "$JAR"
```

判据：

- 状态码 **303**，`Location: ./`；
- 有 `Set-Cookie`，属性包含 `Path=/`、`HttpOnly`、`SameSite=Strict`，**没有 `Secure`**；
- 有效期约 30 天（`Max-Age=2592000` 或等价 `Expires`）；
- cookie 名形如 `dsh-session-<base64url(sha256(authority))>`；
- **再次**访问 `?token=` 应仍能换（幂等）；带错误 token 不行。

### V3 带 cookie 调用 unary RPC（BLOCK 级）

```sh
post() { curl -s -b "$JAR" -H 'Content-Type: application/json' \
  -d "{\"type\":\"client-request\",\"rpcId\":\"$1\",\"method\":\"$2\",\"payload\":{\"args\":$3}}" \
  "$BASE/api/$2"; }

post s1 settings/describe '{}'            | jq .
post s2 session/list '{}'                 | jq '.result.ok, .result.value.items[0]'
post s3 pluginInventory/list '{}'         | jq '.result.ok'
```

判据：HTTP 200，`result.ok = true`；`session/list` 能返回本机真实会话；
响应 `type = "server-response"` 且 `rpcId` 与请求一致。

### V4 WebSocket 升级是否认 cookie（BLOCK 级：决定认证方案）

```sh
# a) 原始握手：期望 101
curl -s -i -N --max-time 3 -b "$JAR" \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H "Sec-WebSocket-Key: $(openssl rand -base64 16)" \
  "$BASE/api/remote.mux" | head -5

# b) 无 cookie：期望 401（而不是 101/404）
curl -s -o /dev/null -w '%{http_code}\n' --max-time 3 \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H "Sec-WebSocket-Key: $(openssl rand -base64 16)" \
  "$BASE/api/remote.mux"
```

判据：a 返回 `101 Switching Protocols`，`upgrade: websocket` 且 `sec-websocket-accept` 正确；
b 返回 401。**若 b 返回 101，说明 WS 不校验 cookie，认证方案需重写。**

### V5 `$events` ready 帧与转发事件（BLOCK 级）

```sh
COOKIE=$(awk '$6 ~ /^dsh-session-/{print $6"="$7}' "$JAR")
wscat -c "ws://127.0.0.1:<port>/api/remote.mux" -H "Cookie: $COOKIE" 2>&1 | tee /tmp/dshg-ws.log
```

在 wscat 里发送：

```json
{"type":"open","streamId":"ev1","endpoint":"$events","payload":{"args":{}}}
```

期望第一帧：

```json
{"type":"item","streamId":"ev1","value":{"type":"ready","clientId":"…","host":{"home":"/Users/…"}}}
```

再触发一个转发事件（另开终端）：

```sh
# 先找可写设置项，再改一个无害字段（字段名以 settings/describe 的 schema 为准）
post s4 settings/describe '{}' | jq '.result.value.namespaces[] | {ns, writable, revision}'
post s5 settings/update '{"ns":"<ns>","patch":{"<field>":<value>},"expectedRevision":<rev>}' | jq .
```

判据：

- ready 帧字段与 PROTOCOL 一致，`clientId` 随重连变化；
- 至少收到 1 个 `{"type":"emit","event":…,"args":[…]}`；
- 未授权时 WS 连接失败（401），与 V4 一致。

### V6 会话创建、发送与 `session/follow` 帧序列（BLOCK 级：渲染正确性）

1. 创建会话（或从 `session/list` 取一个空会话）：

```sh
post c1 session/create "{\"cwd\":\"$WORK\"}" | jq .
```

2. 用 wscat（新连接）打开会话流：

```json
{"type":"open","streamId":"sf1","endpoint":"session/follow","payload":{"args":{"request":{"address":{"kind":"session","sessionId":"<sid>"}}}}}
```

> 形参结构来自类型推断（PROTOCOL §3.1），**若报 `gateway/bad-request`，按错误信息调整**，
> 并把实际可用形状记录到结果里。

3. 观察首帧：期望 `{type:"snapshot", header, cursor, records, hasMore, projections}`。

4. 发送消息（另开终端）：

```sh
post c2 session/prompt "{\"sessionId\":\"<sid>\",\"requestId\":\"r1\",\"mode\":\"queue\",\"content\":[{\"type\":\"text\",\"text\":\"说一句 hi\"}]}" | jq .
```

5. 观察：`assistantStream` 的 `start/chunk/end` 帧、以及 `records` 里 assistant/tool 事件；
   记录 `cursor`、`assistantStream` 出现时机、`end` 后是否还有补帧。

判据：能完整重建一轮对话；帧名/字段与 PROTOCOL §4 一致（允许新增字段）；
断线重开 `session/follow` 能拿到新 snapshot 并接上。

### V7 审批 waterfall 往返（BLOCK 级：审批 UI 的基础）

1. 保持 V6 的 `session/follow` 与 `$events` 两条流都开着（两个 wscat 连接）。
2. 在会话里发一条会触发审批的消息（用 `ask` 权限模式；默认可能已是 ask）：

```sh
post p7 session/prompt '{"sessionId":"<sid>","requestId":"r7","mode":"queue","content":[{"type":"text","text":"用 bash 执行 echo dshg-validate 并报告输出"}]}' | jq .
```

3. 在 `$events` 流上期待 waterfall 帧：

```json
{"type":"waterfall","event":"approval/request","eventId":"…","agentId":"…","request":{"toolName":"bash","reason":"…"}}
```

4. 用 curl 回应（先测允许一次）：

```sh
# outcome: {kind:"result", value:"allowed-once"} / {kind:"result", value:"rejected"}
post a1 '$events/result' '{"clientId":"<clientId>","eventId":"<eventId>","outcome":{"kind":"result","value":"allowed-once"}}' | jq .
```

5. 再发一轮测试拒绝：`"value":"rejected"`，观察会话给出拒绝结果、不卡死。

判据：waterfall 到达；两种 outcome 都能让会话继续/结束；`$events/result` 返回 ok；
记录 `displayReason` 是否在 wire 上、`outcome.kind` 实际取值、是否有超时。

### V8 断线重连与 baseline / gap 修复（BLOCK 级）

1. 会话流正在输出时，直接杀掉 wscat 连接。
2. 重新连 `session/follow`：期望新的 `snapshot`（不是增量续传）。
3. 用 `session/page` 验证 gap 修复：

```sh
post v8 session/page "{\"request\":{\"address\":{\"kind\":\"session\",\"sessionId\":\"<sid>\"},\"throughSeq\":<seq>,\"beforeSeq\":<seq>}}" | jq .
```

判据：重连后事件不重不漏；`session/page` 能按 `throughSeq/beforeSeq` 取到缺的记录；
`$events` 换代后 `clientId` 变化、旧 emit 不重放（符合预期即 PASS\*）。

### V9 插件启停（BLOCK 级：插件面板）

```sh
post p1 pluginInventory/list '{}' | jq '.result.value.entries[0]'
post p2 pluginManager/listPlugins '{}' | jq '.result.value[0]'

# 记录初始状态后，挑一个可回滚的插件
post p3 pluginManager/setPluginEnabled '{"id":"<plugin-id>","enabled":false}' | jq .
post p4 pluginManager/setPluginEnabled '{"id":"<plugin-id>","enabled":true}'  | jq .
```

判据：返回 `ChangeResult.application ∈ {applied, restart-required, overridden, failed, cancelled}`；
记录真实取值；若 `restart-required`，确认重启 dsh 后状态保持；最后还原为启用。

### V10 错误语义与端点探测（PASS 级）

```sh
# 未知端点 → 404
curl -s -o /dev/null -w '%{http_code}\n' -b "$JAR" -H 'Content-Type: application/json' \
  -d '{"type":"client-request","rpcId":"e1","method":"nope/nope","payload":{"args":{}}}' \
  "$BASE/api/nope/nope"

# 非法 JSON → 400
curl -s -o /dev/null -w '%{http_code}\n' -b "$JAR" -H 'Content-Type: application/json' \
  -d '{bad json' "$BASE/api/settings/describe"

# 缺 content-type → 415
curl -s -o /dev/null -w '%{http_code}\n' -b "$JAR" -d '{}' "$BASE/api/settings/describe"

# 业务错误 → 200 + ok:false（如不存在的 session）
post e2 session/page '{"request":{"address":{"kind":"session","sessionId":"does-not-exist"},"throughSeq":0}}' | jq .
```

判据：404 / 400 / 415 / 200+`ok:false` 与 PROTOCOL §2.1、§9 一致；
`method` 与路径不一致时返回 `gateway/bad-request` 错误信封。

### V11（可选）二进制读取

`workspaceFiles/readBytes` 的 multipart 响应：确认 part `metadata` + 字节 part 的结构，
以及大文件是否可接受。不阻塞 M0。

## 5. 影响面（失败时改哪里）

| 失败点 | 影响 | 修订位置 |
| --- | --- | --- |
| V2/V4 认证/升级不符 | attach 方案重写（可能需要每次贴 token） | OUTLINE §5、SPEC §4/§7.1、PROTOCOL §1 |
| V6 帧序列不符 | 渲染/重连策略调整 | PROTOCOL §4、SPEC §5/§7.3 |
| V7 审批不符 | 审批 UI 与超时策略 | PROTOCOL §6、SPEC §7.2 |
| V9 只能重启生效 | 插件面板 UX 改为"修改 + 重启指引" | SPEC §7.4、PARITY |
| V10 错误语义不符 | 错误分类与重试策略 | SPEC §9 |

## 6. 结果记录模板（VALIDATION-RESULTS.md）

```md
# 协议实测结果（YYYY-MM-DD）
- dsh 版本：0.2.0-rc.2
- macOS / 机器：
- 启动命令：dsh --profile web --no-open --port 0

| 用例 | 判定 | 实际行为摘要 | 影响 |
| --- | --- | --- | --- |
| V1 | | | |
| … | | | |

## 差异清单
- …

## 后续动作
- …
```

## 7. 顺序与时间盒

1. V1 → V2 → V3 → V4（约 30 分钟）：认证链路，任何一项 BLOCK 就先停下来改方案。
2. V5 → V6 → V7（约 1.5 小时）：事件流、会话重建、审批。
3. V8 → V9 → V10（约 1 小时）：重连、插件、错误语义。
4. 全部完成后更新 `PROTOCOL.md` 的 `[确认]/[推测]` 标记与 SPEC 的开放问题。
