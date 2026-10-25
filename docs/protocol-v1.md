# WebSocket2Serial 协议 v1（`ws2s/v1`）

本工具是本机常驻的 WebSocket↔串口透传服务，对外暴露两类连接：

- **管理 WebSocket**：`ws://127.0.0.1:<managementPort>/ws`（默认端口 `8765`）。用于
  枚举串口、打开/关闭会话、调试订阅等控制动作，并推送状态事件。
- **数据 WebSocket**：每个串口会话一个，`ws://127.0.0.1:<dataPort>/ws/<token>`，
  `dataPort` 由系统临时分配、`token` 为会话随机令牌。用于串口字节的二进制原样透传。

所有示例与约定固定使用 `127.0.0.1`，**不使用 `localhost`**。

> 兼容性说明：本工具**不做管理鉴权 token**、不做 TLS、不提供 `clientId`；管理 WS
> 只做浏览器 Origin 白名单校验（默认放行 `http://127.0.0.1:*` 与 `http://localhost:*`），
> 数据 WS 仅靠 URL 中的随机 token 识别。数据面是**流式透传**，**不保证报文边界**——
> 一帧二进制消息只是一段字节块，客户端不得假设帧边界与业务报文边界一致。

---

## 1. 消息封装

### 1.1 请求（客户端 → 管理 WS）

```json
{ "id": "任意字符串（用于匹配应答）", "action": "动作名", "params": { } }
```

### 1.2 应答（管理 WS → 客户端）

成功：

```json
{ "id": "...", "ok": true, "data": { } }
```

失败：

```json
{ "id": "...", "ok": false, "error": { "code": "PORT_BUSY", "message": "串口被占用", "detail": { } } }
```

`detail` 为可选字段，仅在能给出平台原始错误时出现。

### 1.3 事件（管理 WS → 客户端）

```json
{ "event": "事件名", "data": { } }
```

事件不带 `id`；请求的应答永远带 `id`。客户端应按「有无 `id`」区分应答与事件，
并按 `id` 关联请求。

---

## 2. 7 个管理动作

### 2.1 `ports.list`

- 参数：`{}`
- 返回：`{ "ports": [PortInfo] }`（立即重扫的当前串口快照）

`PortInfo`（平台拿不到的字段为 `null`）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `path` | string | 串口路径，如 `COM3`、`/dev/ttyUSB0` |
| `portType` | string \| null | 端口类型，如 `usb` |
| `vid` | number \| null | 厂商 ID |
| `pid` | number \| null | 产品 ID |
| `serialNumber` | string \| null | 序列号 |
| `manufacturer` | string \| null | 制造商 |
| `product` | string \| null | 产品名 |

### 2.2 `session.open`

打开串口，或复用同端口尚未 `active` 的会话（见 §5 复用与 token 轮换）。

参数（`OpenParams`，字段为 camelCase）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `path` | string | 串口路径，非空 |
| `baudRate` | number | 波特率，> 0 |
| `dataBits` | number | 数据位，5–8 |
| `parity` | string | `"none"` / `"odd"` / `"even"` / `"mark"` / `"space"`；v1 仅支持前三个 |
| `stopBits` | number | 停止位，1 或 2 |
| `flowControl` | string | `"none"` / `"software"` / `"hardware"` |
| `dtr` | boolean \| 缺省 | 打开后立即施加的 DTR 电平 |
| `rts` | boolean \| 缺省 | 打开后立即施加的 RTS 电平 |

返回（`OpenResult`）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sessionId` | string | 会话 ID（复用时为原 ID） |
| `dataUrl` | string | 数据 WS URL，形如 `ws://127.0.0.1:<port>/ws/<token>` |
| `connectTimeoutMs` | number | 等待数据 WS 连入的时限（默认 5000） |
| `reused` | boolean | 是否复用已有会话（复用会轮换 token） |
| `counters` | Counters | 计数快照 |

`Counters`（64 位）：

| 字段 | 说明 |
|---|---|
| `rxBytes` | 从串口读到的字节总数（含宽限丢弃） |
| `txBytes` | 写入串口的字节总数 |
| `rxDiscardedBytes` | 宽限期内读到但未交付的字节数 |

> `session.open` 后应立即连接 `dataUrl`；`awaiting` 阶段工具不读串口，延迟连入
> 只会把数据压在驱动缓冲里（超时未连入会话被销毁）。

### 2.3 `session.close`

- 参数：`{ "sessionId": string }`
- 返回：`{ "closed": true }`（幂等）

管理端主动关闭会话，数据面以 `4004` 关闭（若数据连接仍存在）。

### 2.4 `session.list`

- 参数：`{}`
- 返回：`{ "sessions": [SessionInfo] }`（按 `sessionId` 升序）

`SessionInfo`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sessionId` | string | 会话 ID |
| `state` | string | `"awaiting"` / `"active"` / `"grace"` / `"closed"` |
| `port` | string | 串口路径 |
| `params` | SessionParams | 参数快照（不含 `path`/`dtr`/`rts`） |
| `createdAt` | number | 创建时间（Unix 毫秒） |
| `dataPort` | number | 数据 WS 端口 |
| `connectBy` | number \| null | awaiting 截止时间（Unix 毫秒） |
| `activeSince` | number \| null | 进入 active 的时间 |
| `graceEndsAt` | number \| null | 宽限截止时间 |
| `counters` | Counters | 计数 |
| `lastGap` | GapInfo \| null | 最近一次断线宽限信息 |
| `lastError` | ErrorBody \| null | 最近一次错误 |
| `lastRxAt` | number \| null | 最近收字节时间 |
| `lastTxAt` | number \| null | 最近发字节时间 |

`SessionParams`：`baudRate`、`dataBits`、`parity`、`stopBits`、`flowControl`。

`GapInfo`：`startedAt`（宽限开始）、`durationMs`（宽限时长）、`discardedBytes`
（宽限内丢弃字节）、`resumed`（是否在宽限内恢复）。

### 2.5 `session.write`（调试写）

- 参数：`{ "sessionId": string, "dataB64": string }`（base64 编码的原始字节，单次 ≤ 64 KiB）
- 返回：`{ "written": number }`

仅 `active` 会话可用；数据在 50ms 内送入「网页 → 串口」队列，队列忙则返回
`LIMIT_EXCEEDED`。**仅用于排障**：调试写与生产数据共用同一串口，可能造成字节交错。

### 2.6 `debug.subscribe`

- 参数：`{ "sessionIds": "all" | [string], "replay": boolean? }`
- 返回：`{ "subscribed": [string] }`（实际订阅到的会话 ID）

`replay: true` 时，先收到会话内存环形缓冲（64 KiB）的回放帧（`replay=true`），
再收实时帧；帧按 `seq` 去重。调试帧可被丢弃（`seq` 可见缺口），不影响正式数据。

### 2.7 `admin.set`

- 参数：`{ "enabled": boolean }`
- 返回：`AdminState`（`{ "enabled": boolean, "url": string }`）

运行时开关管理页服务（**不回写配置**）；成功后会广播一次 `admin.state`。

---

## 3. 5 个事件

### 3.1 `hello`

管理连接建立后立即推送：

| 字段 | 类型 | 说明 |
|---|---|---|
| `protocol` | string | 固定 `"ws2s/v1"` |
| `version` | string | 工具版本 |
| `os` | string | 操作系统 |
| `arch` | string | 架构 |
| `managementPort` | number | 管理 WS 端口 |
| `admin` | AdminState | 管理页状态 |
| `autostart` | boolean | 开机自启状态 |

### 3.2 `ports.changed`

热插拔轮询（每 2s）发现串口变化：

| 字段 | 类型 | 说明 |
|---|---|---|
| `added` | [PortInfo] | 新出现的端口 |
| `removed` | [ { `path`: string } ] | 消失的端口 |

### 3.3 `session.state`

会话状态迁移（`awaiting` / `active` / `grace` / `closed`）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sessionId` | string | 会话 ID |
| `state` | string | 新状态 |
| `port` | string | 串口路径 |
| `params` | SessionParams | 参数 |
| `reason` | string \| 缺省 | 关闭原因（`CLIENT_CLOSED`/`ADMIN_CLOSED`/`DEVICE_LOST`/`WRITE_TIMEOUT`/`SERIAL_IO_ERROR`/`BACKPRESSURE`/`GRACE_TIMEOUT`/`SESSION_EXPIRED`/`SHUTDOWN`） |
| `closeCode` | number \| 缺省 | 对应的数据面 close code（无数据连接时缺省） |
| `graceEndsAt` | number \| 缺省 | 宽限截止时间 |
| `counters` | Counters | 计数 |
| `lastGap` | GapInfo \| null | 最近断线宽限 |
| `lastError` | ErrorBody \| null | 最近错误 |
| `lastRxAt` | number \| null | 最近收字节时间 |
| `lastTxAt` | number \| null | 最近发字节时间 |

### 3.4 `debug.frame`

订阅会话的收发调试帧（仅推送给已 `debug.subscribe` 的连接）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sessionId` | string | 会话 ID |
| `direction` | string | `"rx"`（串口→网页）/ `"tx"`（网页→串口） |
| `seq` | number | 会话内递增序号 |
| `tsMs` | number | 时间戳（Unix 毫秒） |
| `size` | number | 字节数 |
| `dataB64` | string | base64 编码的数据 |
| `replay` | boolean \| 缺省 | 是否为回放帧 |

### 3.5 `admin.state`

管理页服务开关变化：`AdminState`（`{ "enabled": boolean, "url": string }`）。

---

## 4. 错误码与 close code

### 4.1 管理协议错误码（`error.code`，大写蛇形）

| 错误码 | 含义 |
|---|---|
| `INVALID_REQUEST` | JSON 格式或请求结构错误 |
| `UNKNOWN_ACTION` | 未知 action |
| `INVALID_PARAMS` | 参数缺失或取值非法 |
| `PORT_NOT_FOUND` | 指定串口不存在 |
| `PORT_BUSY` | 串口被占用，或同端口已有 `active` 会话 |
| `PERMISSION_DENIED` | 无权限打开串口 |
| `SERIAL_OPEN_FAILED` | 其他打开失败 |
| `SESSION_NOT_FOUND` | 会话不存在或已关闭 |
| `SESSION_STATE` | 当前状态不允许该操作 |
| `LIMIT_EXCEEDED` | 超过最大会话数或队列上限 |
| `INTERNAL` | 未归类的内部错误 |

### 4.2 数据 WS close code

| 代码 | 含义 |
|---|---|
| `1000` | 客户端正常关闭（服务端按 `CLIENT_CLOSED` 释放串口） |
| `1001` | 服务端退出/心跳判死等正常性离开 |
| `1003` | 收到文本帧（数据面只接受二进制） |
| `1009` | 单帧超过上限（默认 1 MiB） |
| `4001` | 串口设备消失（`DEVICE_LOST`） |
| `4002` | 端口被占用 / 会话已 `active`（第二客户端被拒） |
| `4004` | 管理端关闭（`ADMIN_CLOSED`） |
| `4005` | 接收背压（`BACKPRESSURE`） |
| `4007` | 串口读写错误 / 写停滞超时（`SERIAL_IO_ERROR`/`WRITE_TIMEOUT`） |

### 4.3 管理 WS close code

| 代码 | 含义 |
|---|---|
| `1000` | 客户端主动关闭的回显 |
| `1001` | 心跳判死（10s 无 Pong）或离开 |
| `1013` | 写队列满 / 广播落后，服务端强制断开（客户端应重连 + `session.list` 对齐） |

会话关闭原因以数据 WS 的 close code 为准，不要依赖管理事件与数据帧的到达顺序。

---

## 5. 复用与 token 轮换

- `session.open` 遇到同端口已有会话时：
  - `awaiting` / `grace`：返回**原会话**（`reused=true`），**轮换 token**，绑定新的
    数据监听端口并返回新的 `dataUrl`；旧 `dataUrl` 立即失效（握手 404）。
  - `active`：返回 `PORT_BUSY`（每端口同时只有一个活跃数据连接）。
  - `closed`：返回 `SESSION_NOT_FOUND`。
- token 为 128 位随机值的十六进制串，仅随 `session.open` 返回，不出现在 `session.list`。
- 页面刷新/重启后**直接再次 `session.open`**（带同样的串口参数）即可接回；无需等待
  宽限超时，也无需保存会话 ID/token（工具不存储串口参数，参数由客户端侧保存）。

## 6. 客户端接入约定（重连）

1. 页面由本机 `http://127.0.0.1:<port>` 提供；WS 目标用 `127.0.0.1`，不用 `localhost`。
2. 客户端自己保存串口参数；断线/重启后重新 `session.open`。
3. 常驻管理 WS；重连后先 `session.list` 对齐状态，再恢复调试订阅。
4. 数据 WS 断开时，先在宽限期内重连原 `dataUrl`；失败后 `session.open`（复用并轮换
   token），按 200–500ms 起步退避。
5. 页面刷新/浏览器重启后直接 `session.open`；若收到 `PORT_BUSY`（旧连接未释放），退避重试。
6. 不要依赖管理事件与数据帧的到达顺序；会话关闭原因以数据 WS close code 为准。
7. CSP 需允许 `ws://127.0.0.1:*`。

---

## 7. 最小示例

打开串口（管理 WS）：

```json
// 请求
{ "id": "1", "action": "session.open", "params": {
    "path": "COM3", "baudRate": 115200, "dataBits": 8,
    "parity": "none", "stopBits": 1, "flowControl": "none" } }

// 应答
{ "id": "1", "ok": true, "data": {
    "sessionId": "4c8f...", "dataUrl": "ws://127.0.0.1:54321/ws/6b2f...",
    "connectTimeoutMs": 5000, "reused": false,
    "counters": { "rxBytes": 0, "txBytes": 0, "rxDiscardedBytes": 0 } } }
```

连上 `dataUrl` 后，两端以二进制帧透传串口字节；`hello` 事件示例：

```json
{ "event": "hello", "data": {
    "protocol": "ws2s/v1", "version": "0.1.0", "os": "windows", "arch": "x86_64",
    "managementPort": 8765,
    "admin": { "enabled": true, "url": "http://127.0.0.1:8766" },
    "autostart": false } }
```
