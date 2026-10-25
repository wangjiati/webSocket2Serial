# WebSocket2Serial 透传工具设计（精简版）

- 日期：2026-10-25
- 状态：已批准（2026-10-25）
- 目标平台：Windows 7 SP1 及以上（v1 仅 x64）、Ubuntu 18.04 及以上（x86_64）

## 1. 背景与意图

浏览器里的 Web Serial API 每次使用都要弹窗让用户选择串口。工业现场设备重启后没人操作，网页无法自动连上串口。

本工具作为本机常驻的串口适配器：

- 网页通过 WebSocket 连接本机的透传工具，间接读写串口。
- 串口参数全部由网页下发，透传工具不持久化任何串口参数。
- 一个固定的管理 WebSocket 端口负责扫描、打开、关闭、状态与调试；每个已打开的串口再分配一个独立的数据 WebSocket 端口，只搬运字节。
- 自带一个可开关的本地网页管理界面（串口助手风格）；Windows 常驻托盘显示状态，Linux 托盘可选。
- 用户登录后自启；现场工控机已确认配置开机自动登录。
- 透传工具的定位是“USB 转串口适配器的软件等价物”：如实反映串口状态，不替网页做业务决策，不自动重开、不缓存、不重放。

## 2. 目标、非目标与反目标

### 2.1 目标

1. 网页刷新、浏览器重启、工控机重启后，无需人工授权即可恢复串口通信。
2. 原始字节双向透传，顺序不变；数据面除 WebSocket/TCP 本身外不做任何改写。
3. 管理面与数据面端口分离：管理端口固定，每个串口会话的数据端口独立。
4. 同一个串口同时只允许一个数据客户端，其他连接明确拒绝。
5. 断开、掉线、溢出、被占用等故障都有明确原因和计数，做到“不静默丢失”。
6. 本地管理页提供类似串口助手的实时收发查看、回放与调试发送。
7. 在 Windows 7 SP1 x64 与 Ubuntu 18.04 上可构建、可运行。

### 2.2 非目标

- 不做局域网/互联网访问。v1 只绑定 `127.0.0.1`。
- 不做串口协议解析（不切 Modbus 帧、不做 3.5 字符静默判断）。
- 不做串口参数持久化、开机自动打开串口。
- 不做报文级详细日志、抓包回放、数据落盘。
- 不做用户体系、多租户、远程升级。
- 不提供原生桌面窗口；管理界面是本地网页。
- 不承诺物理层/驱动层零丢包；承诺“应用层不静默丢包，丢了能看见”。
- 不做进程崩溃自动拉起；需要时由操作系统 supervisor 负责。
- v1 不提供 Windows 安装器与 Linux .deb，先交付免安装包。

### 2.3 反目标（即使技术上做得到也算失败）

- 为了“自动恢复”而在透传工具里偷偷缓存或重放串口数据。
- 串口会话在网页崩溃后长期占住端口，必须人工重启工具才能释放。
- 管理页关闭后无法再打开（必须保留管理 WebSocket 和启动参数这两条后路）。
- 为了跨网络访问而牺牲本机免配置启动，或引入证书安装步骤。

## 3. 已确认的约束与决策

| 项 | 决定 | 来源 |
|---|---|---|
| 部署形态 | 后台进程 + 本地网页管理 + Windows 托盘（Linux 可选） | 用户 + 建造者 |
| 访问范围 | 仅 `127.0.0.1`，跨网络暂缓 | 用户 |
| 串口参数 | 网页下发，透传工具不存储 | 用户 |
| 数据面 | 二进制原样透传，不保证报文边界 | 用户 |
| 并发 | 每个串口仅一个数据客户端，其余拒绝 | 用户 |
| 网页断线 | 10 秒宽限期，期间串口保持、数据丢弃并计数 | 用户 |
| 串口掉线 | 立即销毁会话并如实上报，让网页决定重试；不自动重开 | 用户 |
| 管理页开关 | 运行时可关，不持久化；启动默认开；`--admin` 可强制开 | 建造者 |
| 调试界面 | 串口助手式实时收发 + 小缓冲回放，不落盘明细 | 用户 |
| 日志 | 仅会话级基础事件，按天保留 7 天 | 用户 + 建造者 |
| 自启 | 登录后自启；现场工控机已确认开机自动登录 | 用户 |
| 技术栈 | Rust | 用户建议 |
| 最低系统 | Windows 7 SP1 x64、Ubuntu 18.04 x86_64 | 用户 |
| 生产网页来源 | 本机 `http://127.0.0.1:<port>` | 用户 |
| 管理页助手 | 旁观已打开会话 + 调试写，不自行开串口 | 用户 |
| 故障容忍 | 丢字节、端口占死、网页连不回，均不可接受 | 用户 |

## 4. 总体架构

```
┌──────────────────────────── 工控机（127.0.0.1） ────────────────────────────┐
│                                                                             │
│  生产网页 / 管理页                                                          │
│      │  管理 WebSocket（JSON）        数据 WebSocket（纯二进制）            │
│      │  ws://127.0.0.1:8765/ws       ws://127.0.0.1:<动态端口>/ws/<token>   │
│      ▼                                      ▼                               │
│  ┌──────────────────────────────────────────────────────────────┐           │
│  │ WebSocket2Serial 进程                                        │           │
│  │  ├─ 管理 WS 服务（常开，不可关闭）                            │           │
│  │  ├─ 会话管理器（独占、生命周期、计数、错误映射）              │           │
│  │  ├─ 每会话：串口读线程 / 写线程 / 数据 WS 监听器              │           │
│  │  ├─ 本地 HTTP 管理服务（可开关，默认 8766）                    │           │
│  │  ├─ 串口热插拔轮询（2s）                                      │           │
│  │  └─ 托盘（Windows 原生；Linux 可选）                          │           │
│  └──────────────────────────────────────────────────────────────┘           │
│                  │                                                          │
│                  ▼                                                          │
│            COM3 / /dev/ttyUSB0 …                                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 端口布局

| 服务 | 默认地址 | 是否可关 | 说明 |
|---|---|---|---|
| 管理 WebSocket | `127.0.0.1:8765/ws` | 否 | 固定端口，生产网页与管理页都用它做控制/状态 |
| HTTP 管理页 | `127.0.0.1:8766` | 是（运行时） | 只提供静态页面、JS/CSS 与 `/api/config`；关闭后管理 WS 仍可用 |
| 数据 WebSocket | `127.0.0.1:0`（系统分配） | 随会话 | `session.open` 时绑定，返回完整 URL；会话结束即关闭 |

- 所有监听硬编码 `127.0.0.1`，配置不提供 `bind`，避免误开 `0.0.0.0`。
- 文档、页面、数据 URL 一律使用 `127.0.0.1`，不要使用 `localhost`，避免 Win7 上 localhost 先解析到 `::1`。
- 管理端口始终常开，是关闭管理页之后的恢复入口；托盘或 `--admin` 也能重新打开管理页。

### 4.2 会话状态机

```
                 session.open
                      │
                      ▼
                ┌───────────┐  5s 内无数据客户端   ┌────────┐
                │ awaiting  │ ───────────────────▶ │ closed │
                └───────────┘                      └────────┘
                      │ 数据 WS 带 token 连入
                      ▼
                ┌───────────┐  客户端正常关闭(1000)  ┌────────┐
                │  active   │ ────────────────────▶ │ closed │
                └───────────┘                        └────────┘
                   │      ▲
       异常断开     │      │ 10s 内重连（旧 URL 或再次 open）
                   ▼      │
                ┌───────────┐  超时                    ┌────────┐
                │  grace    │ ───────────────────────▶ │ closed │
                └───────────┘                          └────────┘
                      │ 串口设备消失 / 读写失败 / 写超时
                      ▼
                    closed（立即，不等待宽限）
```

- `awaiting`：串口已打开、数据监听已绑定，**不启动读线程**；等待网页连入，超时 5s 后关闭串口并回收端口。此期间设备发来的数据留在操作系统缓冲区，网页连入后按原样交付（不会吞掉设备首帧）。如果设备在此期间持续发送并撑爆驱动缓冲，那部分丢失属于驱动层，工具无法计数；5s 窗口用于限制这种情况。
- `active`：数据 WebSocket 已绑定，读/写线程启动，双向透传。
- `grace`：网页异常断开，串口保持打开，读线程继续读但数据丢弃并计数；期间允许旧 URL 重连或再次 `session.open`（会轮换 token）。超时 10s 后销毁会话。
- 任何串口设备消失、读写 I/O 错误、写超时：立即关闭数据 WS（4001/4007）并销毁会话，不自动重开。

## 5. 管理协议 v1

### 5.1 传输与消息格式

- 地址：`ws://127.0.0.1:8765/ws`
- 全部为 UTF-8 JSON 文本帧；单条消息上限 1 MiB。
- 客户端请求：`{"id":"<字符串>","action":"<动作>","params":{...}}`
- 服务端应答：`{"id":"<同上>","ok":true,"data":{...}}` 或 `{"id":"<同上>","ok":false,"error":{"code":"...","message":"...","detail":...}}`
- 服务端事件：`{"event":"<事件名>","data":{...}}`，无 `id`。
- `id` 由客户端生成，只需在单条连接内唯一。
- 连接建立后服务端立即推送 `hello`。
- 允许同时存在多个管理连接；管理连接断开不影响已建立的串口会话。
- 状态事件与调试帧分开投递：状态事件队列有界，某个管理连接消费过慢、队列溢出时，服务端以 1013 关闭该连接，客户端重连后调用 `session.list` 重新对齐；调试帧允许丢弃，客户端用 `seq` 察觉缺口。
- 管理 WS 与数据 WS 是两个独立连接，事件与数据帧的到达顺序不保证；会话关闭以数据 WS 的 close code 为准，管理事件作为补充。

`hello` 示例：

```json
{
  "event": "hello",
  "data": {
    "protocol": "ws2s/v1",
    "version": "0.1.0",
    "os": "windows",
    "arch": "x86_64",
    "managementPort": 8765,
    "admin": {"enabled": true, "url": "http://127.0.0.1:8766/"},
    "autostart": true
  }
}
```

### 5.2 动作

v1 只有 7 个动作：

| action | params | 返回 | 说明 |
|---|---|---|---|
| `ports.list` | 无 | `{ports:[PortInfo]}` | 立即重扫并返回当前串口快照 |
| `session.open` | `OpenParams` | `{sessionId,dataUrl,connectTimeoutMs,reused,counters}` | 打开串口或复用同端口的非 active 会话；复用会轮换 token |
| `session.close` | `{sessionId}` | `{closed:true}` | 幂等；关闭串口并回收端口 |
| `session.list` | 无 | `{sessions:[SessionInfo]}` | 不返回 `dataUrl`/token |
| `session.write` | `{sessionId,dataB64}` | `{written}` | 仅调试用；仅 `active` 会话；单次上限 64 KiB |
| `debug.subscribe` | `{sessionIds:["..."]|"all",replay?:true}` | `{subscribed:[...]}` | 替换订阅集；`[]` 清空；`replay` 回放内存环形缓冲 |
| `admin.set` | `{enabled:true|false}` | `{enabled,url}` | 运行时开关管理页服务，不写配置文件 |

- `debug.subscribe` 的 `"all"` 覆盖当前及后续新建会话；显式 ID 列表只覆盖列出的会话，未知 ID 忽略。

#### OpenParams

```json
{
  "path": "COM3",
  "baudRate": 115200,
  "dataBits": 8,
  "parity": "none",
  "stopBits": 1,
  "flowControl": "none",
  "dtr": true,
  "rts": false
}
```

- `path`：Windows 为 `COM3`（`COM10` 及以上同样写 `COM10`，内部转换）；Linux 为 `/dev/ttyUSB0`。
- `baudRate`：整数，>0；允许非标准值（如 250000），由平台驱动决定是否支持。
- `dataBits`：5/6/7/8。
- `parity`：`none` / `odd` / `even` / `mark` / `space`。v1 实际仅支持 `none`/`odd`/`even`；`mark`/`space` 协议值保留，但 `SerialParams::validate()` 会直接返回 `INVALID_PARAMS`（serialport 4.10.1 限制）。
- `stopBits`：1 或 2（v1 不支持 1.5）。
- `flowControl`：`none` / `software` / `hardware`。
- `dtr`、`rts`：可选，打开后立即应用的初始电平；缺省为驱动默认。
- 不提供 `clientId`。会话归属按端口判定，规则见下。

#### 会话复用规则

1. 该 `path` 无会话 → 新建，状态 `awaiting`，生成随机 token，返回 `dataUrl`。
2. 该 `path` 已有会话，状态为 `awaiting` 或 `grace` → **不重新打开串口**，轮换 token，返回原 `sessionId`、新的 `dataUrl`、`reused:true` 和当前计数。旧 URL 立即失效。
3. 该 `path` 已有会话且状态为 `active` → `PORT_BUSY`。

含义：

- 网页刷新/浏览器重启后，新页面直接调用 `session.open`，即可接回原会话（如果状态已进入 `awaiting`/`grace`）。若旧数据连接尚未完全断开，会短暂收到 `PORT_BUSY`，页面按 200–500ms 退避重试即可。
- 页面若仍持有旧 `dataUrl`，也可以先直接重连；成功后无需 `session.open`。一旦再次 `session.open`，旧 token 失效，以最新一次 open 为准。
- 不做客户端身份、不依赖 localStorage 保存 session id。

#### PortInfo

```json
{
  "path": "COM3",
  "portType": "usb",
  "vid": 1027,
  "pid": 24577,
  "serialNumber": "A123",
  "manufacturer": "FTDI",
  "product": "USB Serial"
}
```

非 USB 或平台拿不到描述时，相应字段为 `null`。

#### SessionInfo

```json
{
  "sessionId": "01J...",
  "state": "active",
  "port": "COM3",
  "params": {"baudRate":115200,"dataBits":8,"parity":"none","stopBits":1,"flowControl":"none"},
  "createdAt": 1760000000000,
  "dataPort": 54321,
  "connectBy": 1760000010000,
  "activeSince": 1760000005000,
  "graceEndsAt": null,
  "counters": {
    "rxBytes": 10240,
    "txBytes": 512,
    "rxDiscardedBytes": 0
  },
  "lastGap": null,
  "lastError": null,
  "lastRxAt": 1760000009000,
  "lastTxAt": 1760000008000
}
```

- `rxBytes`：从串口读到的总字节数（含宽限期丢弃的）；`awaiting` 阶段尚未开始读，计为 0。
- `rxDiscardedBytes`：宽限期内读到但未交付的字节数。
- 交付字节数无需单独下发，恒为 `rxBytes - rxDiscardedBytes`。
- `txBytes`：写入串口的字节数，包含调试写。
- 不提供 `rxChunks`/`txChunks`，减少无意义字段。
- `lastGap`：`{startedAt,durationMs,discardedBytes,resumed}`，保留最近一次断线信息。

### 5.3 事件

v1 只有 5 个事件：

| event | data | 触发 |
|---|---|---|
| `hello` | `{protocol,version,os,arch,managementPort,admin:{enabled,url},autostart}` | 管理连接建立 |
| `ports.changed` | `{added:[PortInfo],removed:[{path}]}` | 热插拔轮询发现变化 |
| `session.state` | `{sessionId,state,port,params,reason?,closeCode?,graceEndsAt?,counters,lastGap,lastError?,lastRxAt?,lastTxAt?}` | 会话状态迁移 |
| `debug.frame` | `{sessionId,direction:"rx"|"tx",seq,tsMs,size,dataB64,replay?}` | 已订阅会话的收发帧 |
| `admin.state` | `{enabled,url}` | 管理页服务开关 |

- `session.state` 的 `state` 为 `awaiting` / `active` / `grace` / `closed`。
- `closed` 时带 `reason` 与 `closeCode`，例如 `CLIENT_CLOSED`/`GRACE_TIMEOUT`/`DEVICE_LOST`/`WRITE_TIMEOUT`/`SERIAL_IO_ERROR`/`ADMIN_CLOSED`/`BACKPRESSURE`/`SERVER_SHUTDOWN`。
- `grace` 时带 `graceEndsAt`；重新进入 `active` 时 `lastGap` 与 `counters` 反映这次断线丢弃量。
- 不再单独发送 `session.grace` / `session.resumed` / `session.attached` / `session.closed`，全部由 `session.state` 表达。
- `debug.frame.seq` 为会话内单调递增序号；序号跳变说明调试事件被丢弃或缓冲被截断。
- 管理连接积压时不再发送 `lagged` 事件，直接断开让客户端重连对齐。

### 5.4 错误码

| code | 含义 |
|---|---|
| `INVALID_REQUEST` | JSON 格式或请求结构错误 |
| `UNKNOWN_ACTION` | 未知 action |
| `INVALID_PARAMS` | 参数缺失或取值非法 |
| `PORT_NOT_FOUND` | 指定串口不存在 |
| `PORT_BUSY` | 串口已被本工具或其他程序占用；或同端口已有 active 会话 |
| `PERMISSION_DENIED` | 无权限打开（Linux 常见于不在 dialout 组） |
| `SERIAL_OPEN_FAILED` | 其他打开失败，`detail` 带系统错误 |
| `SESSION_NOT_FOUND` | 会话不存在或已关闭 |
| `SESSION_STATE` | 当前状态不允许该操作（如非 active 时调试写） |
| `LIMIT_EXCEEDED` | 超过最大会话数或队列上限 |
| `INTERNAL` | 未归类的内部错误 |

`error.detail` 允许包含平台原始错误码与文本，便于现场排查。

## 6. 数据面协议

- 地址：`ws://127.0.0.1:<ephemeral>/ws/<token>`，由 `session.open` 返回。
- **只接受二进制帧**。客户端→服务端二进制帧即写入串口；服务端→客户端二进制帧即串口读到的字节块。文本帧以 1003 关闭。
- 一条帧对应一次串口读取，最大 4096 字节；**不保证报文边界**，网页自行缓冲拼帧。
- token 为 128 位随机值的十六进制，只出现在 `dataUrl` 中；每次 `session.open` 复用会话时轮换，旧 token 立即失效。
- 错误 token 返回 HTTP 404，不泄露会话是否存在。
- 第二个数据连接：握手成功后在应用层以 4002 关闭；不打扰已有连接。
- 服务端启用 TCP_NODELAY；不做压缩。
- 客户端单条消息上限 1 MiB，超过以 1009 关闭。
- 会话状态、错误、计数全部通过管理端口通知；数据端口内不混入 JSON。

### 6.1 Close codes

| code | 名称 | 说明 |
|---|---|---|
| 1000 | Normal | 网页主动正常关闭，立即销毁会话并释放串口 |
| 1001 | Going Away | 工具退出或系统关闭 |
| 1003 | Unsupported Data | 收到文本帧 |
| 1009 | Message Too Big | 客户端帧超过 1 MiB |
| 4001 | SERIAL_DEVICE_LOST | 串口设备消失（USB 拔出、掉电） |
| 4002 | PORT_BUSY | 已有数据客户端 |
| 4004 | SESSION_CLOSED_BY_ADMIN | 通过管理端关闭 |
| 4005 | BACKPRESSURE | 接收队列积压超限，主动失败而不是静默丢弃 |
| 4007 | SERIAL_IO_ERROR | 串口读写错误或写超时 |

- 不再保留实际不会出现的 `SERIAL_OPEN_FAILED`、`SESSION_EXPIRED`、`SESSION_NOT_FOUND` 数据面 close code；这些情况通过握手 404 或管理事件表达。

### 6.2 计数与“不静默丢失”

每个会话维护 64 位计数：

- `rxBytes`：从串口读到的总字节数（`active` + `grace`；`awaiting` 尚未读）。
- `rxDiscardedBytes`：`grace` 阶段读到的、未交付的字节数。
- 已交付字节 = `rxBytes - rxDiscardedBytes`。
- `txBytes`：写入串口的字节数（数据 WS 与调试写都计入）。

不变式：`rxBytes >= rxDiscardedBytes`，且交付量 = `rxBytes - rxDiscardedBytes`。测试在物理回环中核对对端发送量与桥接计数；驱动层损失无法由工具感知，单独记录为测试异常，不计入应用层计数。

- `awaiting` 阶段不读，数据留在 OS 缓冲；网页连入后正常交付，不会吞首帧。若 OS 缓冲在此期间溢出，属于驱动层损失，工具无法感知，5s 连接超时用于限制窗口。
- `grace` 阶段继续读并丢弃，避免网页重连后先收到一堆积压的过期应答。
- 重连成功后，管理端 `session.state` 的 `lastGap.discardedBytes` 与计数可用于判断错过了多少、是否需要重新初始化设备。
- 物理层/驱动层丢包无法由本工具感知；本工具保证的是应用层不主动静默丢包，任何主动丢弃都有计数和事件。

### 6.3 背压策略

- 串口→网页：每会话接收队列上限 1 MiB（可配）。队列满说明网页长时间不读或本机严重卡顿；此时以 4005 关闭会话并上报，**不继续阻塞读串口**（阻塞会导致驱动缓冲溢出且无人知晓）。
- 网页→串口：每会话发送队列上限 1 MiB（可配）。队列满时停止读取 WebSocket（由 TCP 窗口对浏览器施加背压），不丢字节；若客户端持续不消费，连接会卡住而非丢数据。
- 调试帧是旁路复制：调试订阅过载时允许丢弃调试事件，绝不影响正式数据通道；`seq` 缺口用于提示。

## 7. 关键运行行为

### 7.1 串口扫描与热插拔

- 启动后立即扫描一次，此后每 2s 轮询系统串口列表，按 `path` 做差集，推送 `ports.changed`。
- `ports.list` 会立即重扫，不等下一轮轮询。
- 不使用平台专有热插拔 API，保证 Win7/Ubuntu 18.04 行为一致；代价是事件延迟最多约 2s。
- 串口正在使用时设备被拔出：读写立即报错，会话按 4001 销毁，同时推送 `ports.changed` 的 removed。

### 7.2 独占

- 应用层：同一 `path` 同时只允许一个会话；打开前检查并加锁。
- 数据层：一个会话只接受一个数据 WS，token 唯一且可轮换。
- 系统层：依赖驱动默认的独占打开行为；若驱动允许共享，仍以应用层为准。

### 7.3 心跳与宽限

- 管理 WS 与数据 WS 均由服务端每 5s 发 WebSocket Ping；10s 内无 Pong 视为死连接。
- 浏览器会自动回 Pong。
- 数据连接判定死亡后按“异常断开”进入 10s 宽限期；最坏端口占用约 20s（10s 判死 + 10s 宽限）。localhost 上 TCP 关闭通常是即时的，这条慢路径只用于浏览器卡死等极端情况。
- 若现场需要更快释放，可把宽限期调小；默认 10s 符合用户确认值。

### 7.4 网页正常关闭 vs 异常断开

- 收到 close code 1000：网页明确表示结束，立即销毁会话、释放串口。
- 收到其他 close code、TCP 直接断开、心跳超时：进入宽限期。
- 工具退出（托盘退出、SIGTERM）：所有数据 WS 以 1001 关闭，串口全部释放，然后退出；不做宽限。

### 7.5 串口写超时

- 每个会话的写线程按约 100ms 线路时间分块写入（块大小随波特率调整，256–4096 字节），每块使用 `write_timeout_ms`（默认 5s）作为“停滞超时”。
- 若某一块连续 5s 无进展（设备不读、硬件流控阻塞、驱动异常），立即以 4007 / `WRITE_TIMEOUT` 销毁会话，释放端口，而不是无限挂住。
- 写超时是“停滞”而不是“总时长”：大块数据只要持续有进展就可以写完，不会因为总时长超过 5s 被误杀。
- M0 需验证 `serialport` 在当前平台是否支持写超时；若不支持，用独立写线程 + 看门狗兜底。

### 7.6 启动与退出

启动顺序：

1. 解析配置、初始化日志。
2. 单实例检查；已有实例则尝试打开其管理页并退出。
3. 绑定管理 WS 端口（`127.0.0.1:8765`）。失败视为致命错误：写日志，Windows 弹 MessageBox，Linux 写 stderr。
4. 按配置（或 `--admin`）启动 HTTP 管理服务；失败不致命，记录状态并在托盘/管理端口体现。
5. 启动托盘、热插拔轮询。
6. 不打开任何串口。

退出时先停止接受新连接，再关闭所有会话（1001），刷日志，退出。会话不持久化，下次由网页重新下发。

### 7.7 单实例

- Windows：命名互斥量（当前用户作用域）。
- Linux：`$XDG_RUNTIME_DIR/websocket2serial.lock` 文件锁，记录 PID。
- 第二实例不启动服务，只尝试打开已有实例的管理页；失败则提示（Windows MessageBox/托盘气泡，Linux 日志）。

### 7.8 自启、自动登录与进程守护

- 自启：Windows 当前用户注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`；Linux `~/.config/autostart/websocket2serial.desktop`。
- 现场已确认工控机开机自动登录；否则“登录后自启 + 托盘”无法满足重启自动恢复，需要改为服务/计划任务并接受无托盘，二者需现场取舍。
- 启用自启时记录当前可执行文件路径；每次启动若路径变化则更新注册表/.desktop，避免免安装包移动后失效。
- **v1 不做进程崩溃自动拉起**。若现场要求，使用操作系统 supervisor：Windows 计划任务（失败重启）/ Linux systemd user 服务（`Restart=on-failure`），不要自己写守护进程。这是已知边界，需在部署文档中写明。

## 8. 安全

v1 的信任边界是“本机”。不提供远程访问，不做用户体系。

- 所有监听硬编码 `127.0.0.1`；配置不暴露 `bind`，避免误开 `0.0.0.0`。
- WebSocket 握手校验 `Origin`，默认允许：
  - `http://127.0.0.1:*`
  - `http://localhost:*`
  - v1 不支持 `file://` 页面（`Origin: null`）。生产网页必须由本机 HTTP 提供。
  - 可在配置中追加精确 Origin；不允许单独的 `*`（端口通配 `:*` 可以）。
- 不启用管理鉴权 token：本机信任边界下，Origin 白名单已能挡住远程网页；鉴权 token 只会增加网页部署配置项。本机恶意进程本来就可以直接连端口，token 防不住。数据面的随机 token 只用于绑定会话，不是鉴权。
- 不启用 TLS：本机回环不承担证书与兼容性成本。
- 调试写 `session.write` 只允许本机管理连接，且要求会话处于 `active`；管理页默认不勾选“允许写入”，发送前二次确认，并明确提示“会与网页写入共用串口”。
- 日志不记录报文内容，也不记录 `dataUrl`/token。
- `127.0.0.1` 是唯一地址；不要用 `localhost`，避免 IPv6 解析问题。
- 生产网页若有 CSP，需要允许 `connect-src ws://127.0.0.1:*`（数据端口是动态端口，需通配端口）。
- Windows 防火墙可能首次监听时弹窗；回环连接通常不受影响，部署时预放行或忽略即可。

## 9. 管理页与托盘

### 9.1 管理页

- 单页应用，纯 HTML/CSS/JavaScript，无构建步骤、无外部 CDN，资源编译进二进制。
- 兼容 Windows 7 上的 Chrome 109 与 Firefox 115 ESR（ES2020 范围）。
- 页面通过管理 WS 获取数据，不直接打开业务数据端口；串口助手只旁路订阅生产网页已经打开的会话，并可用 `session.write` 调试发送，**不能自行打开或独占串口**。没有活动会话时助手为空。
- 页面从同源 HTTP 接口 `GET /api/config` 获取管理端口、版本、管理页状态、自启状态；WS 目标固定为 `ws://127.0.0.1:<managementPort>/ws`，不使用 `location.hostname`，避免 localhost/IPv6 问题。

页面结构：

1. **概览**
   - 工具版本、管理地址 + 复制按钮、管理服务开关（关闭前确认并提示刷新后无法访问）。
   - 串口列表（路径、USB 厂商/产品/序列号），支持刷新。
   - 会话列表（端口、参数、状态、计数、最近错误），可关闭会话。
   - 管理服务关闭后页面失联；托盘可重新打开，或重启时加 `--admin`。
2. **串口助手**
   - 选择会话，订阅调试帧（可回放最近 64 KiB）。
   - 接收区：序号、时间戳（毫秒）、方向 RX/TX、HEX/ASCII 切换、自动滚动、暂停、清空。
   - 发送区：HEX/文本输入，经 `session.write` 发送；默认禁用，需勾选“允许写入”并二次确认，显示共用串口警示。
   - 显示会话状态、宽限剩余时间、close code、计数、`seq` 缺口提示。
   - “导出当前缓冲”按钮：客户端生成文本文件下载，不常驻落盘。
3. **设置/关于**
   - 只读展示配置路径与关键参数（宽限期、队列上限、最大会话数、Origin 白名单）。
   - 修改配置需编辑文件并重启。

调试缓冲：

- 每个会话在内存保留最近 64 KiB 的收发帧（只存原始字节与元数据，不落盘），每帧带 `seq`。
- `debug.subscribe` 不带 `replay` 时只看后续实时帧；带 `replay:true` 先补发缓冲。序号跳变说明缓冲被截断或调试事件被丢弃。
- 管理页自身最多保留 5000 帧或 4 MiB，超出丢弃最旧帧并提示；导出只导出客户端当前缓冲。

关于“关闭管理页省内存”：

- 静态 HTTP 服务本身占用很小，真正占内存的是浏览器页面。这个开关主要减少入口和误操作，不承诺显著降低进程内存；仍按用户要求保留。

### 9.2 托盘

- Windows：用 Win32 `Shell_NotifyIcon` 直接实现，避免引入 GUI 框架，保证 Win7 可用。
- Linux：托盘不作为 v1 交付门槛。默认构建不包含托盘；有需要时用可选 feature 编译 AppIndicator 版本，桌面不支持则降级为无托盘，管理页与协议功能不受影响。
- 图标状态：
  - 灰色：无会话
  - 绿色：至少一个 `active`
  - 黄色：有 `awaiting` 或 `grace`
  - 红色：最近一次会话因错误关闭
- Tooltip：版本、管理端口、会话数、最近错误；例如“WebSocket2Serial · 2 个会话 · COM3 115200 · 最近错误: DEVICE_LOST”。
- 菜单：状态行（禁用项）、打开管理页、管理服务开/关、开机自启开/关、关于、退出。
- 双击：打开管理页；若管理服务被关闭，则先启动再打开。
- 串口掉线、打开失败、背压关闭时弹托盘气泡（`NIF_INFO`），不依赖管理页就能看到。
- 不再提供“复制管理地址”托盘项；管理页概览里有复制按钮。

### 9.3 自启与自动登录

- 安装/首次部署时开启自启；托盘菜单可切换。
- Windows：当前用户注册表 `HKCU\...\Run`；Linux：`~/.config/autostart/websocket2serial.desktop`。
- 现场工控机必须开启自动登录；这是“重启后网页自动连回”的前提。部署清单必须包含这一项。
- 自启仅为“登录后启动进程”；不打开串口，不恢复会话。

### 9.4 生产网页接入约定

供生产网页重写和参考客户端使用：

1. 页面由本机 `http://127.0.0.1:<port>` 提供；WS 目标使用 `127.0.0.1`，不要用 `localhost`。
2. 页面自己保存串口参数（工具不存）；断线/重启后重新 `session.open`。
3. 常驻管理 WS；重连后先调用 `session.list` 对齐状态，再恢复调试订阅。
4. 数据 WS 断开时，先在宽限期内尝试原 `dataUrl`；失败后调用 `session.open`，按 200–500ms 起步退避。
5. 页面刷新/浏览器重启后直接 `session.open`；若收到 `PORT_BUSY`，说明旧连接尚未释放，退避重试。
6. 不要依赖管理事件与数据帧的到达顺序；会话关闭原因以数据 WS close code 为准。
7. CSP 允许 `ws://127.0.0.1:*`。

## 10. 配置与日志

### 10.1 配置

- Windows：`%APPDATA%\WebSocket2Serial\config.toml`
- Linux：`$XDG_CONFIG_HOME/websocket2serial/config.toml`，缺省 `~/.config/websocket2serial/config.toml`
- 首次启动生成默认配置。解析失败时记录错误并使用默认值启动（保证能进管理页修复），托盘显示警告。
- **串口参数绝不写入配置**；只有工具自身设置会持久化。

```toml
[server]
management_port = 8765

[admin]
enabled = true
port = 8766

[session]
connect_timeout_ms = 5000
grace_ms = 10000
max_sessions = 8
read_timeout_ms = 5
write_timeout_ms = 5000
rx_queue_bytes = 1048576
tx_queue_bytes = 1048576

[security]
allowed_origins = ["http://127.0.0.1:*", "http://localhost:*"]

[logging]
level = "info"
keep_days = 7
```

- 不提供 `bind`、管理鉴权 `token`。
- `admin.enabled` 只决定启动时是否开启管理页；`admin.set` 只改运行时状态，**不回写配置**。
- 命令行：`--config <path>`、`--admin`（强制本次启动开启管理页）、`--version`。

### 10.2 日志

只记录基础事件，不记录收发内容：

- 启动/退出、版本、配置路径、监听地址绑定结果。
- 管理页服务开启/关闭。
- 会话打开（端口、波特率、数据位、校验、停止位、流控）、关闭（原因、close code、计数摘要）。
- 数据客户端连接、断开、宽限开始/恢复/超时。
- 调试订阅、调试写（仅字节数）。
- 所有错误及平台原始错误信息。

路径：Windows `%LOCALAPPDATA%\WebSocket2Serial\logs\`，Linux `~/.local/state/websocket2serial/logs/`。

轮转：按天轮转，保留 7 天（`tracing-appender` 的 daily + `max_log_files`）。不自行实现按大小轮转。

## 11. 技术选型与工程结构

### 11.1 运行时与依赖

- Rust，单 crate 单二进制。
- `tokio`：多线程运行时、TCP/WS、定时器、广播事件。
- `axum`（含 WebSocket）：管理 WS、数据 WS、HTTP 管理页。使用 `axum::serve` 或自定义 accept 循环以设置 `TCP_NODELAY`。
- `serialport`：跨平台串口枚举与读写。
- `serde` / `serde_json` / `toml`：协议与配置。
- `tracing` / `tracing-subscriber` / `tracing-appender`：基础日志（按天轮转）。
- `base64`、`thiserror`、`uuid`（v4）。
- 静态资源用 `include_str!` / `include_bytes!` 内嵌，不引入 `rust-embed`。
- `open`：打开默认浏览器。
- Windows 托盘：`windows-sys` 直接调用 `Shell_NotifyIconW`、`CreateWindowExW`、`TrackPopupMenu`。
- Linux 托盘（可选 feature）：`libappindicator3`；不可用则编译/运行降级，不阻塞核心功能。
- 不引入 TLS、数据库、GUI 框架、前端框架、剪贴板库（复制地址由管理页 JS 完成）。
- 不引入 `arboard`：避免 headless Linux/X11 依赖。

### 11.2 模块划分

```
src/
  main.rs                启动、单实例、CLI、信号处理
  config.rs              TOML 配置、默认值、路径
  logging.rs
  protocol/
    mod.rs               管理协议 DTO
    errors.rs            错误码与映射
  serial/
    mod.rs               串口枚举、热插拔轮询
    port.rs              打开/配置/读写封装、写超时
    params.rs            参数校验
  session/
    manager.rs           独占锁、会话表、生命周期、token 轮换
    session.rs           awaiting/active/grace 状态、计数、环形调试缓冲
  ws/
    management.rs        管理 WS
    data.rs              数据 WS
  http_admin.rs          静态资源、/api/config、管理页开关
  tray/
    mod.rs
    windows.rs
    linux.rs
  platform/
    autostart.rs
    single_instance.rs
    paths.rs
  assets/                index.html / app.js / style.css / icons
clients/
  ws2serial-client.js    最小参考客户端（无构建步骤）
examples/
  ws2serial-cli.py       现场/验收用命令行示例
tests/
  protocol_codec.rs
  session_lifecycle.rs   基于假串口 Transport
  ws_integration.rs      真实回环 + PTY/com0com（按平台忽略）
```

### 11.3 串口 I/O 模型

- 数据 WS 连入（`awaiting → active`）时才启动读/写线程；`awaiting` 阶段不读，保留设备首帧。
- 每个会话两个阻塞线程：读线程、写线程，共享 `Arc<Mutex<Box<dyn SerialPort>>>`。
- 读线程用 5ms 超时循环读取，读到数据立即投递到接收队列；无数据时让出锁。
- 写线程把客户端帧按约 100ms 线路时间分块（256–4096 字节），每块使用写停滞超时；超时销毁会话。
- 锁竞争最坏引入约 5ms 延迟；对常见 Modbus/文本协议足够。若实测不达标，再改为平台事件驱动（epoll / overlapped I/O），不改变外部协议。
- 所有跨线程队列有界；队列满按 6.3 处理。
- `grace` 阶段读线程继续运行、丢弃并计数；写线程空闲。
- 串口参数变化只能通过关闭后重新打开；v1 不支持运行中改波特率。
- 关闭会话时先停读/写线程（带超时），再释放串口句柄。

### 11.4 构建与兼容

**Windows 7 SP1**

- 使用 Rust 1.77.2（最后一个官方支持 Win7 的稳定版本），目标 `x86_64-pc-windows-msvc`，开启 `-C target-feature=+crt-static`，避免安装 VC 运行库。
- 锁定 `Cargo.lock`，防止依赖升级抬高 MSRV 或引入 Win7 不可用 API。
- 若某个必要依赖要求更高 rustc，则退回夜间版 `x86_64-win7-windows-msvc` tier-3 目标 + `-Z build-std`；此路线作为备选，不作为首选。
- 交付前必须在 Win7 SP1 x64 虚拟机实测启动、串口、管理页、托盘、写超时。v1 不构建 32 位；现场若确认存在 32 位 Win7，再评估 i686 构建与验收。

**Ubuntu 18.04**

- 在 `ubuntu:18.04` 容器内构建，保证 glibc 2.27 兼容。
- 安装 `build-essential`、`pkg-config`、`libudev-dev`；运行时依赖 `libudev1`（系统自带）。
- Linux 托盘是可选 feature，默认构建不启用；启用时额外依赖 `libappindicator3-dev` 与运行时 `libappindicator3-1`，桌面不支持则无托盘。

**打包（v1 先简单交付）**

- Windows：免安装 zip；解压到固定目录，托盘菜单开启自启。Inno Setup 安装包推迟到现场验证稳定后再做。
- Linux：tar.gz + `install.sh`（复制到 `~/.local/bin`、生成绝对路径的 `.desktop` 自启项、可选托盘包）。.deb 推迟。
- 静态资源内嵌，运行时不需要额外文件。
- 代码签名非 v1 必需；若现场杀软拦截，再评估签名。

### 11.5 可观测性

- 管理端口 `session.list` 与 `hello` 提供运行态。
- 托盘 tooltip 提供最简状态与计数。
- 基础日志用于事后确认“何时开了哪个口、为什么关”。
- 不提供 Prometheus/远程上报。

## 12. 验收与测试

### 12.1 功能验收

1. 冷启动工具，管理端口可用，串口列表正确；管理页可通过 `http://127.0.0.1:8766/` 打开。
2. 网页下发参数打开串口，拿到数据 URL，5s 内连上后双向收发正确。
3. 第二个数据连接被 4002 拒绝，原连接不受影响。
4. 同端口：`active` 时再次 `session.open` 返回 `PORT_BUSY`；`awaiting`/`grace` 时返回原会话并轮换 token，旧 URL 失效。
5. 网页正常关闭 → 串口立即释放，可被其他程序打开。
6. 网页异常断开 → 10s 内重连成功，`lastGap.discardedBytes` 正确；超时后串口释放。网页刷新/浏览器重启后通过再次 `session.open` 直接接回，无需等宽限超时。
7. 设备打开后先发 banner：`session.open` 后延迟 200ms 再连数据 WS，仍能收到 banner（`awaiting` 不读）。
8. USB 拔出 → 数据 WS 立即 4001，会话销毁，端口释放；插回后 `ports.changed` 通知，网页可重新打开。
9. 管理 WS 断开不影响正在工作的数据连接；重连后 `session.list` 能对齐状态。
10. 管理页可开关；关闭后管理 WS 仍可重新开启；`--admin` 可在重启后强制打开；托盘也能开关。
11. 管理页串口助手可实时看 HEX/ASCII、暂停/清空、回放、导出、调试发送；不能自行打开串口；调试写需勾选确认。
12. Win7 登录自启与托盘正常；Ubuntu 18.04 登录自启正常，托盘按可选 feature 验收（缺失时管理页可用）。
13. 进程退出后所有串口句柄释放，无残留端口占用。
14. 串口写停滞（设备不读/流控阻塞）→ 5s 内以 4007/WRITE_TIMEOUT 销毁会话，端口可重新打开。
15. 统一使用 `127.0.0.1` 连接；不使用 `localhost` 也能在 Win7 正常连通。
16. 工控机重启并自动登录后，桥和网页按自启/重连逻辑恢复，无需人工授权。
17. 串口被其他程序占用 → `PORT_BUSY`；Linux 无权限 → `PERMISSION_DENIED`。

### 12.2 故障与压力验收

| 场景 | 通过标准 |
|---|---|
| 115200 物理回环持续 30 分钟 | 收发字节计数与对端完全一致，无 4005 |
| 8 小时浸泡 | RSS 稳定（波动 <5%），无会话/线程泄漏 |
| 数据连接反复断开/重连 50 次 | 每次宽限/恢复计数正确，最终端口可释放 |
| 页面刷新 50 次 | 每次都能通过 `session.open` 接回，旧 token 失效 |
| 设备拔插 20 次 | 4001、端口释放、插回通知、重开成功 |
| 921600 回环持续 10 分钟（可选） | 硬件具备时执行，不作 v1 交付门槛 |
| 921600 下管理页订阅调试（可选） | 正式数据不受影响；调试事件允许丢弃，`seq` 可见缺口 |
| 发送洪泛超过串口能力 | 连接变慢或 4005，绝不静默丢 TX |
| 接收端暂停读取 | 队列达上限后 4005，计数可解释 |
| 串口被其他程序占用 | `PORT_BUSY`，不崩溃 |
| 无权限（Linux 非 dialout） | `PERMISSION_DENIED`，提示明确 |
| 写停滞（设备不读/流控阻塞） | 5s 内 4007，端口可重新打开 |
| 工具强杀后重启 | 串口可重新打开，网页重连成功 |
| 管理页开关 20 次 | 运行时可反复开关，`--admin` 可恢复，管理 WS 不受影响 |

环境说明：当前已具备 Win7 SP1 虚拟机、Ubuntu 18.04 环境、可拔插 USB 转串口，且工控机已配置开机自动登录；com0com/socat 需在 M0 安装，或使用 TX-RX 短接的物理回环。当前没有 921600 链路，相关用例标记为可选，不阻塞 v1 交付。

### 12.3 延迟目标

- 工具内部延迟（串口读到数据 → 交给 WebSocket；WebSocket 收到 → 写入串口）：p99 ≤ 5ms。
- 端到端 115200 物理回环另测，受 USB 转串口芯片与驱动 latency timer 影响；目标 p99 ≤ 25ms，仅作观察指标，不作为 v1 交付门槛。
- 若 FTDI 等驱动默认 latency timer 为 16ms，需在部署说明中提示可调低；这属于驱动配置。
- 延迟测试用对端打时间戳、回环回显，并在工具内部埋点分别统计。

### 12.4 测试工具与环境准备

- Linux：`socat` 创建 PTY 对；Python + pyserial 做回显、计数、时间戳。当前环境需安装 `socat`。
- Windows：com0com 虚拟串口对；Python + pyserial；Win7 SP1 虚拟机。当前需在虚拟机内安装 com0com。
- 物理回环：USB 转串口 TX-RX 短接（或两只能互连的 USB 串口），用于真实驱动路径的计数与延迟测试。
- Rust 单元/集成：用 `Transport` trait 注入假串口，覆盖状态机、宽限、掉线、计数、背压、写超时。
- 故障注入脚本：杀数据连接、关 PTY、拔 USB、灌数据、开第二连接。
- 协议验收：用 `clients/ws2serial-client.js` 与 `examples/ws2serial-cli.py` 各跑一遍，确认协议文档与实现一致。
- 浏览器：Win7 上 Chrome 109 与 Firefox 115 ESR 各跑一遍管理页。

## 13. 风险与待验证

| 风险 | 影响 | 应对 |
|---|---|---|
| Rust 1.77.2 与所需依赖版本冲突 | Win7 构建失败 | 依赖锁版本；备选 nightly + `win7` tier-3 目标；再不行评估 Go 1.20（最后一个支持 Win7 的 Go） |
| Win7 托盘与消息循环实现 | 托盘不可用 | M0 先验证；失败则降级为无托盘 + 管理页 |
| `serialport` 写超时在平台不生效 | 写线程可能挂住 | M0 验证；不生效则独立写线程 + 看门狗/关闭句柄兜底 |
| 串口读写共用句柄的延迟 | 写入最多等 5ms | 实测；不达标改平台事件驱动 |
| Ubuntu 18.04 托盘支持 | 无托盘 | 不进入 v1 验收门槛；可选 feature，缺失时用管理页 |
| 验收环境缺少 com0com/socat 与 921600 | 部分故障/高速用例无法执行 | M0 安装工具或做 TX-RX 物理回环；921600 降为可选 |
| Win7 32 位需求 | 构建矩阵扩大 | v1 仅 x64；现场盘点后如需要再增加 i686 构建与测试 |
| 进程崩溃后无人拉起 | 网页持续重试但服务不恢复 | v1 已知边界；需要时用 Windows 计划任务 / Linux systemd user，不自写守护进程 |
| 预连接期间驱动缓冲溢出 | `awaiting` 阶段数据在工具外丢失 | 连接超时 5s；网页应 open 后立即连数据 WS；文档说明 |
| USB 驱动 latency timer | 端到端延迟偏大 | 部署文档给出驱动调优建议 |
| 生产网页 CSP | 动态数据端口被拦截 | 文档要求 `connect-src ws://127.0.0.1:*` |
| Windows 防火墙首次监听弹窗 | 首次启动被询问 | 回环通常不受影响；部署时预放行或忽略 |
| 旧浏览器（Chrome 109 / Firefox 115 ESR） | 管理页兼容性 | 只用 ES2020；M0/M4 在两个浏览器实测 |
| RS485 自动方向控制 | 部分适配器不可用 | v1 只提供打开时的 DTR/RTS 初值；运行中切换与自动方向后续评估 |

## 14. 里程碑

**M0 可行性 spike（一次性，不进入产品）**

- 环境准备：Win7 SP1 虚拟机安装 com0com；Ubuntu 18.04 安装 socat；制作 USB 转串口 TX-RX 回环（或确认可用回显设备）。
- Rust 1.77.2 构建 Win7 x64 可执行文件，在 Win7 SP1 虚拟机打开串口、跑通最小 WS 回环、显示托盘、弹气泡。
- 验证 `serialport` 写超时/写停滞行为；验证 `127.0.0.1` 与 `localhost` 在 Chrome 109 下的差异。
- Ubuntu 18.04 容器构建并运行同一最小程序。
- 不测延迟（spike 无计时仪器）；记录测量方案，真实实现任务 7 内置两处计时，M2/M4 再测 115200 回环延迟与吞吐；921600 视硬件条件选做。
- 结论决定是否调整工具链、托盘方案、串口 I/O 模型。

**M1 核心通路**

- 配置、日志、单实例、CLI、管理 WS、串口扫描/热插拔、`session.open/close/list`（含复用与 token 轮换）、数据 WS 原始透传、计数、明确错误码。
- 验收：功能验收 1–5、8–9、13、15、17。

**M2 可靠性**

- 10s 宽限与 gap 计数、设备掉线、写超时、背压、心跳、管理连接独立性、管理页运行时开关、退出清理。
- 验收：功能验收 6–7、14、16 与故障矩阵。

**M3 管理页与托盘**

- 内嵌管理页（概览、串口助手、回放/导出、调试写保护）、`/api/config`、托盘（气泡/tooltip）、自启。
- 交付 `docs/protocol-v1.md`、`clients/ws2serial-client.js`、`examples/ws2serial-cli.py`。
- 验收：功能验收 10–12。

**M4 打包与现场验证**

- Windows zip、Ubuntu tar.gz/install.sh、部署文档（自动登录、CSP、127.0.0.1、FTDI 调优、崩溃守护边界）。
- Win7 SP1 与 Ubuntu 18.04 实机/虚拟机全量验收、8 小时浸泡、50 次重连、自动登录重启验证。
- 921600 与 Ubuntu 托盘作为可选验收项，硬件/桌面具备时执行。

## 15. 决策归属

### 15.1 用户已确认

- 本机 `127.0.0.1` 访问，跨网络暂缓。
- 串口参数由网页管理，工具不存储。
- 数据面二进制原样透传。
- 每串口独占，其他客户端拒绝。
- 断线 10 秒宽限；串口掉线如实上报、由网页决定重试。
- 管理页可开关，管理 WS 常开作为恢复入口。
- 调试为串口助手式实时展示，不落盘明细；日志仅基础事件。
- 管理页助手只旁观/调试写，不自行打开串口。
- Win7 登录后自启、带托盘；Ubuntu 托盘可选。
- Win7 SP1、Ubuntu 18.04 最低版本；v1 仅 x64；Rust。
- 生产网页由本机 `http://127.0.0.1:<port>` 提供。
- 工控机已配置开机自动登录。
- 丢字节、端口占死、网页连不回均不可接受。

### 15.2 建造者决定（可在评审时推翻）

- 管理端口默认 8765，管理页默认 8766。
- 数据端口由系统临时分配，每个会话一个；用含随机 token 的 URL 绑定；复用会话时轮换 token。
- 默认值：连接超时 5s、宽限 10s、收发队列各 1 MiB、最大会话 8、读超时 5ms、写停滞超时 5s、热插拔轮询 2s、心跳 5s/10s。
- 协议面收敛为 7 个动作、5 个事件、3 个计数 + `lastGap`；不提供 `clientId`、管理鉴权 token、`ports.rescan`、`system.info`、`admin.status`、`session.set_signals`。
- `awaiting` 不读；`grace` 继续读并丢弃计数；去掉 `rxDiscardedAwaitingBytes` 与 `session.attached`。
- 管理页运行时可关、不持久化；`--admin` 强制开启；`admin.enabled` 只作启动默认。
- 管理页允许 `session.write` 调试发送，但默认关闭、需勾选并二次确认；每会话保留 64 KiB 调试环形缓冲 + `seq` + replay；客户端可手动导出，不落盘。
- 管理事件对慢消费者直接断开（1013）；调试事件可丢，`seq` 提示缺口。
- Windows 采用 Rust 1.77.2 + 静态 CRT，仅 x64；Ubuntu 在 18.04 容器内构建。
- Linux 托盘不进入 v1 交付，作为可选 feature；缺失时用管理页。
- Origin 默认只放行 `http://127.0.0.1:*` 与 `http://localhost:*`；v1 不支持 `file://`；不做管理鉴权 token、不做 TLS；硬编码 `127.0.0.1`。
- 日志按天轮转保留 7 天；不按大小轮转。
- v1 交付 zip/tar + 自启开关；Windows 安装器/.deb 推迟。
- 交付协议文档、参考 JS 客户端、Python CLI 示例。
- 单实例、托盘菜单项与状态色。
- v1 不做进程崩溃自动拉起，使用 OS supervisor 作为可选部署手段。

### 15.3 后续现场确认

- 现场是否存在 32 位 Windows 7；v1 先按 x64 交付。
- 921600 链路何时具备；具备后补跑可选的高速验收。
- Ubuntu 托盘是否要从“可选”升级为硬需求。

### 15.4 已知边界

- 管理服务关闭后，Linux 无托盘的恢复手段是 `--admin` 重启或管理 WS；管理页本身不可用。
- `awaiting` 阶段工具不读串口，若设备持续发送并撑爆驱动缓冲，损失发生在驱动层，工具无法计数。
- 调试帧是旁路、可丢；正式数据通道不丢（除非驱动/物理层）。
- 管理页调试写会与生产网页共用同一串口，可能造成字节交错；仅用于排障。
- 进程崩溃不会自动重启；需要时由 OS supervisor 负责。
- 不做跨网络、TLS、报文切帧、自动重开、串口参数持久化。
