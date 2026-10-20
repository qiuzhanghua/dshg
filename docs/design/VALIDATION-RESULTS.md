# 协议实测结果（2026-10-20）

- 环境：macOS，dsh `@deepseek-ai/dsh` 0.2.0-rc.2（DSH Tauri 内置依赖），Node v24.19.0
- 启动：绕过桌面 shim，直接调用
  `node .../@deepseek-ai/dsh/lib/bin.js --profile web --no-open --port 0`，
  `DSH_HOME=/Users/daniel/space/research/dshg/.dsh`（隔离，不碰真实会话）
- 实测地址：`http://127.0.0.1:52631`（端口随机）
- 结论：**V1–V5 全部 PASS，无 BLOCK**；M0 的认证与 WS 方案可继续。

## 用例结果

| 用例 | 判定 | 实际结果 |
| --- | --- | --- |
| V1a 无 cookie GET `/` | PASS | 401 |
| V1b 错误 token | PASS | 401 |
| V1c 无 cookie 调 API | PASS | 401 |
| V2 token → cookie | PASS | `303 See Other`，`location: ./`，`Set-Cookie` 属性齐全 |
| V3 unary（带 cookie） | PASS | `settings/describe` → `ok:true` |
| V3 session/list | PASS\* | 首参必须叫 `_request`，否则 `gateway/arguments-invalid` |
| V4a 带 cookie WS 升级 | PASS | `101 Switching Protocols` + 正确 `Sec-WebSocket-Accept` |
| V4b 无 cookie WS 升级 | PASS | 401 |
| V5 `$events` ready | PASS | 收到 ready 帧（见下） |

## 关键实测证据

### V2 的 Set-Cookie

```
set-cookie: dsh-auth-Z7QivDIbBx8xNqPnxV3cOsZSzOc83UP-QS3BviON668=v1.eyJ...;
            Max-Age=2592000; Path=/; Expires=Thu, 19 Nov 2026 10:52:58 GMT;
            HttpOnly; SameSite=Strict
```

- cookie 名是 **`dsh-auth-<hash>`**，不是 PROTOCOL 里推测的 `dsh-session-*`；
- 无 `Secure`，30 天，`HttpOnly`、`SameSite=Strict`，与源码一致；
- curl 的 cookie jar 会把该行写成 `#HttpOnly_127.0.0.1 ...`，解析时要处理前缀。

### V4/V5

```
GET /api/remote.mux  -> 101 Switching Protocols
（带 cookie；无 cookie 为 401）

{"type":"open","streamId":"ev1","endpoint":"$events","payload":{"args":{}}}
-> {"type":"item","streamId":"ev1","value":{"type":"ready",
     "clientId":"779bb156-636a-4c82-abab-798bb9be4bb7",
     "host":{"home":"/Users/daniel"}}}
```

用 Node（v24）的全局 `WebSocket` 即可带 Cookie 头完成升级与开流：

```js
new WebSocket(url, { headers: { Cookie: cookie } })
```

### V3 参数命名

- `session/list` 的形参名是 `_request`：`payload: {args: {_request: {}}}` 才成功；
- `settings/describe`、`pluginInventory/list` 用空 `args: {}` 即可；
- 返回 `session/list` → `{items: []}`（隔离 home 下无历史会话）。

## 需要回写的文档

1. `PROTOCOL.md` §1.1：cookie 名前缀改为 `dsh-auth-`（已实测）；补"V1–V5 已实测"标记。
2. `VALIDATION.md`：
   - V3 的 `session/list` 示例补 `_request`；
   - V2 的 cookie 名匹配改为 `dsh-auth-`；
   - V4/V5 增加 Node `WebSocket` 带 headers 的做法；
   - 补"绕过 `~/.local/bin/dsh` shim"的说明（shim 硬编码 `DSH_HOME=~/.dsh`）。
3. `M0-DESIGN.md`：`session/list` 请求带 `_request`；Keychain 条目不变。

## 端到端验证（dshg --probe）

T1–T3 合并后，用 `./target/debug/dshg --probe "<url>"` 对真实 dsh 跑通：

```
[info] authenticated with dsh authority=127.0.0.1:53520 name=dsh-auth-<hash>
[]
```

`[]` 是隔离 home 下的 `session/list` 结果；说明 URL 解析、token 交换、cookie 携带、
unary 信封与 `_request` 参数全部正确。

## 未覆盖（M0 不阻塞）

- V6 `session/follow` 帧序列、V7 审批往返、V8 重连 gap、V9 插件启停：需要真实模型与工具调用，
  留到 M1/M2 前的专项实测。
- V10 错误语义、V11 二进制读取：未跑（低风险）。

## 复现命令摘要

```sh
NODE="/Applications/DSH Tauri.app/Contents/Resources/resources/node/bin/node"
BIN="/Users/daniel/Library/Application Support/dsh-tauri/dependencies/dsh/node_modules/@deepseek-ai/dsh/lib/bin.js"
DSH_HOME="$PWD/.dsh" "$NODE" "$BIN" --profile web --no-open --port 0
# 记录输出里的 http://127.0.0.1:<port>/?token=<t>

BASE="http://127.0.0.1:<port>"; TOKEN="<t>"; JAR=/tmp/dshg-cookies.txt
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/"                     # 401
curl -s -i -c "$JAR" "$BASE/?token=$TOKEN" | head                     # 303 + Set-Cookie
curl -s -b "$JAR" -H 'Content-Type: application/json' \
  -d '{"type":"client-request","rpcId":"1","method":"session/list","payload":{"args":{"_request":{}}}}' \
  "$BASE/api/session/list"                                            # ok:true
```
