# 串口助手 WS 报文观察与串口刷新实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 subagent-driven-development（推荐）或 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在串口助手页按时间平铺显示每个操作对应的管理/数据 WebSocket 报文与生命周期事件，并提供手动重新扫描全部串口的按钮。

**架构：** 在共享 `ManagementClient` 与 `SerialSession` 中增加可选、异常隔离的只读 trace 钩子；串口助手页把两条通道的事件统一加上时间戳，存入独立的 2000 条环形上限缓冲并渲染。刷新按钮复用现有 `ports.list`，服务端已经会立即重扫串口，无需修改 Rust 产品代码。

**技术栈：** 原生 HTML/CSS、零构建 ES modules、浏览器 WebSocket API、Rust 静态资源断言测试。

**规格：** `docs/superpowers/specs/2026-10-27-assistant-ws-trace-design.md`（执行者必须同时阅读）

## 全局约束

- 零构建、无 CDN、无外部依赖；兼容 Chrome 109 / Firefox 115 ESR。
- 不新增或修改管理协议动作、事件、close code、会话生命周期行为；不修改 Rust 产品代码。
- 管理 WS 的 JSON 必须显示线上原文：不重排、不 pretty print、不省略字段。
- 报文按时间顺序平铺，每条显示操作类型；不按操作分组。
- 只修改串口助手页；管理页、功能测试页和独立演示页不增加这些控件。
- trace 回调必须异常隔离，不能改变现有连接、重连、收发、超时行为。
- WS 报文缓冲上限为 **2000 条**；收起或暂停时仍捕获到内存，超出丢弃最旧记录。
- 数据 WS 的二进制发送只记录字节数，串口负载仍由原有串口日志显示。
- HTML 使用现有 Rust 静态资源控制点测试；JS 行为复用仓库已有的可选 Node 运行方式，只用 Node 内置模块执行真实源码，不引入 npm、jsdom 或新的测试依赖。

## 审查重点（Review Focus）

1. **trace 回调抛异常：** 正常 `ws.send`、入站分发、数据 WS 关闭和重连仍应完成；任务 1 的异常隔离契约测试与实现检查覆盖。
2. **自动 `session.list` 轮询在面板收起/暂停时持续到达：** 内存仍按时间记录但 DOM 不增长，恢复后能看到记录且总数不超过 2000；任务 3 的 `pushCapped` 行为测试和手工暂停、收起检查覆盖。
3. **`ports.list`、`session.list` 等请求并发且回应乱序：** 每条回应按 `id` 显示原请求的 action，不错标为最后发出的 action；任务 1 的乱序回应行为测试与任务 3 的打开串口往返检查覆盖。
4. **管理 WS 断开时手动刷新，且输入框已有手工串口号：** 显示失败/未连接提示，不伪报成功，不覆盖输入；任务 2 的成功/失败返回值行为测试和真实页面检查覆盖。
5. **原文 JSON 或错误文本含 `<img>`、`<script>` 等字符：** 报文只作为文本显示，不成为 HTML；任务 3 的恶意串口名真实页面检查覆盖。

---

## 文件结构

- 修改 `src/assets/js/api.js`：管理 WS 原文/连接 trace 钩子；`refreshPorts()` 返回本次是否成功。
- 修改 `src/assets/js/serial.js`：数据 WS 连接、发送、关闭 trace 钩子。
- 修改 `src/assets/js/assistant-page.js`：串口助手 trace 状态、渲染、暂停/清空/导出，以及刷新按钮行为。
- 修改 `src/assets/js/format.js`：提供面板实际使用的 `pushCapped(items, item, limit)` 有界数组辅助函数。
- 修改 `src/assets/assistant.html`：刷新按钮和默认收起的 WS 报文面板。
- 修改 `src/assets/style.css`：WS 报文行、面板高度、操作类型与方向样式。
- 修改 `tests/admin_assets.rs`：新 DOM 控制点，以及调用 Node 行为测试的 Rust 测试入口。
- 创建：`tests/js/assistant-behavior.test.mjs`：用 Node 内置断言和假 WebSocket 执行真实 `api.js`/`serial.js`，验证 trace 与刷新行为。
- 修改 `README.md`：串口助手能力说明。

### 任务 1：共享 WebSocket trace 钩子

**文件：**
- 修改：`src/assets/js/api.js`
- 修改：`src/assets/js/serial.js`
- 测试：`tests/admin_assets.rs`

**接口：**

管理面 trace entry 统一为：

```js
{ channel: "mgmt", dir: "out" | "in" | "event", kind: string, text: string }
```

数据面 entry 的 `channel` 为 `"data"`，其余字段相同。此时不附 `ts`，由任务 3 的页面统一加时间戳。`ManagementClient.onTrace(cb)` 与现有 `onEvent(cb)` 一样只注册、不返回；`SerialSession` 通过 `hooks.onTrace` 接收。

- [ ] **步骤 1：编写失败的行为测试**

创建 `tests/js/assistant-behavior.test.mjs`，用 `node:assert/strict` 和 Node 内置模块执行真实源码，不测试源码文本：

- `api.js` 无相对 import，可通过 `data:` URL 动态 import；`serial.js` 与 `format.js` 复制到临时目录为 `.mjs`，只把 import 路径改为 `./format.mjs` 后 import。测试结束清理临时目录。
- 假 WebSocket 只替代浏览器外部边界，提供 `OPEN=1`、构造器、`send`、`close` 和事件回调；真实 `ManagementClient`/`SerialSession` 逻辑不替换。
- trace 用例：并发发出 `ports.list` 与 `session.list`，按相反顺序回应，断言原始发送字符串与 trace 文本完全一致，回应分别标为 `ports.list ok`、`session.list ok`；无效 JSON 产生 `kind=invalid` 且不抛出；首个 trace 回调抛错时第二个仍收到记录，请求/回应仍完成。
- 数据面用例：实际调用 `connectData()`、`sendBytes(Uint8Array.of(1,2,3))` 和 `ws.onclose`，断言依次出现 `data.connecting`、`data.open`、`data.send`（正文 `3 B`）、`data.close`（含实际 code/reason）；hook 抛错时发送和 frames 记录仍完成。
- 脚本以 `process.argv[2]` 选择 `trace`、`refresh`、`panel` 或默认 `all`（任务 2/3 逐步补充对应用例）。

在 `tests/admin_assets.rs` 增加 Node 运行辅助函数：Node 不可用时按仓库现有可选测试惯例跳过，否则运行 `node tests/js/assistant-behavior.test.mjs <case>`，失败时打印 stdout/stderr 并 assert。新增 `websocket_trace_hooks_capture_raw_traffic_safely` 调用 `trace` 用例。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets websocket_trace_hooks_capture_raw_traffic_safely -- --exact`

预期：FAIL，Node 行为测试首先因 `ManagementClient.onTrace` 尚不存在（或后续真实 trace 行为缺失）而失败；Rust 包装测试打印该 stdout/stderr。

- [ ] **步骤 3：实现 `ManagementClient` 管理面 trace**

在 `src/assets/js/api.js` 中：

1. 构造器增加 `this.traceHandlers = new Set()`；增加 `onTrace(cb)` 和 `emitTrace(entry)`。`emitTrace` 遍历处理器，每个回调单独 `try/catch`，异常只忽略。
2. `request(action, params)` 只生成一次 `const raw = JSON.stringify(payload)`；pending 等待项增加 `action` 字段；调用 `this.ws.send(raw)` **成功后**发出 `{channel:"mgmt",dir:"out",kind:action,text:raw}`。发送抛错时不伪造已发送记录，现有清理和 reject 不变。
3. `handleMessage(raw)` 保留原始字符串。JSON 解析失败时发 `dir:"in",kind:"invalid",text:String(raw)` 后按原逻辑返回。解析成功后、事件/回应分发前发出入站 trace：
   - 事件：`kind = ` 模板 `event:${msg.event}`。
   - 有 `id` 的回应：通过 `this.pending.get(msg.id)?.action` 找原 action；找到时 kind 为 `action + " ok"` 或 `action + " error"`，找不到时为 `response`。
   - 其他合法 JSON：kind 为 `message`。
4. `ws.onopen` 发 `kind:"connect"`，正文为管理 WS URL；`ws.onclose` 在 `handleClose()` 前发 `kind:"disconnect"`，正文含 `code=<code>`，有 reason 时附上 reason。

- [ ] **步骤 4：实现 `SerialSession` 数据面 trace**

在 `src/assets/js/serial.js` 中：

1. hooks JSDoc 增加 `onTrace?:(entry:object)=>void`；增加 `_trace(dir, kind, text)`，内部以 `try/catch` 调用 `this.hooks.onTrace?.({channel:"data",dir,kind,text})`。
2. `connectData()` 创建 `WebSocket` 前记录 `data.connecting`，正文为 dataUrl 最后一个 `/` 后的部分；`ws.onopen` 记录 `data.open`。
3. `sendBytes(bytes)` 在 `this.ws.send(bytes)` 成功后记录 `dir:"out",kind:"data.send",text:`${bytes.length} B``；不复制负载到 trace。
4. `ws.onclose` 记录 `dir:"event",kind:"data.close"`，正文含 `code=<code>` 与非空 reason；不得改变现有 state、grace、重连与 waiter 逻辑。

- [ ] **步骤 5：运行测试验证通过并检查回归**

运行：`cargo test --test admin_assets`

预期：PASS，包括新增行为测试和该文件原有测试。Node 行为测试已覆盖回调异常隔离，无需另做 DevTools 钩子检查。

- [ ] **步骤 6：Commit**

```bash
git add src/assets/js/api.js src/assets/js/serial.js tests/admin_assets.rs tests/js/assistant-behavior.test.mjs
git commit -m "feat: add websocket trace hooks"
```

### 任务 2：手动刷新全部串口

**文件：**
- 修改：`src/assets/js/api.js`
- 修改：`src/assets/js/assistant-page.js`
- 修改：`src/assets/assistant.html`
- 测试：`tests/admin_assets.rs`

**接口：**

`ManagementClient.refreshPorts()` 改为返回 `Promise<boolean>`：更新 `this.ports` 并 `notifySync()` 后为 `true`，原有 catch 保留旧数据并返回 `false`。已有调用方忽略返回值，行为不变。

- [ ] **步骤 1：编写失败的测试**

在 `tests/js/assistant-behavior.test.mjs` 增加 `refresh` 用例，直接对真实 `ManagementClient.refreshPorts()` 提供可控的 `request()` 替身：

- 成功返回 `{ports:[{path:"COM7"}]}` 时，结果为 `true`，`client.ports` 更新，且 sync 回调执行一次。
- 请求抛错时，结果为 `false`，刷新前的 `client.ports` 保持不变。

在 `tests/admin_assets.rs` 增加 `management_refresh_ports_reports_success` 调用 Node `refresh` 用例；另增加 `assistant_can_refresh_ports_on_demand`，断言 HTML 含 `id="assistant-refresh-ports"` 和按钮文本 `刷新串口`。不要对 JS 源码字符串做断言；按钮提示和输入保护由步骤 6 的真实页面检查验收。

- [ ] **步骤 2：运行测试验证失败**

分别运行：

- `cargo test --test admin_assets management_refresh_ports_reports_success -- --exact`
- `cargo test --test admin_assets assistant_can_refresh_ports_on_demand -- --exact`

预期：前者因现有 `refreshPorts()` 成功时返回 `undefined` 而非 `true` 失败；后者因缺少 `assistant-refresh-ports` 失败。

- [ ] **步骤 3：实现可报告结果的端口刷新**

1. `src/assets/js/api.js`：按上述接口让 `refreshPorts()` 返回布尔值，不改变 catch 时保留旧列表的行为。
2. `src/assets/js/assistant-page.js`：现有页面级 `refreshPorts(client)` 在未连接时返回 `false`，否则返回 `await client.refreshPorts()` 的结果；渲染 `client.ports` 的现有位置不变。`renderPorts` 的 `if (!$("#assistant-port").value && ports[0])` 保护不得删除。
3. 新增 `async function refreshPortsFromButton(client)`：未连接时显示 `管理 WS 未连接，无法刷新串口`；否则禁用按钮、显示 `正在刷新串口…`，执行 `const ok = await refreshPorts(client)`，成功显示 `已刷新，检测到 ${client.ports.length} 个串口`，失败显示 `刷新串口失败，请稍后重试`，在 `finally` 中恢复按钮。
4. `bindUI(client)` 绑定按钮点击到 `refreshPortsFromButton(client)`；回调内部处理结果，不产生未处理 rejection。

- [ ] **步骤 4：添加刷新按钮**

在 `src/assets/assistant.html` 的串口号 `datalist` 后添加：

```html
<button id="assistant-refresh-ports" type="button">刷新串口</button>
```

按钮使用现有表单网格和按钮样式，不增加第三方图标或新依赖。`ports.list` 的服务端实现已经立即重扫（`src/ws/management.rs`），不要增加新的 HTTP API。

- [ ] **步骤 5：运行自动测试**

运行：`cargo test --test admin_assets`

预期：PASS。

- [ ] **步骤 6：手工验证刷新与输入保护**

1. 打开 `/assistant`，在串口框输入一个列表中不存在的名字，点击「刷新串口」；预期输入不被覆盖，结果区显示检测到的串口数，datalist 更新。
2. 管理 WS 未连接时点击按钮；预期显示未连接提示，不显示成功数量。
3. 有 USB 串口时插拔后点击按钮；预期无需刷新网页即可看到新增/移除项。没有硬件时记录为未执行，不伪造结果。

- [ ] **步骤 7：Commit**

```bash
git add src/assets/js/api.js src/assets/js/assistant-page.js src/assets/assistant.html tests/admin_assets.rs tests/js/assistant-behavior.test.mjs
git commit -m "feat: add manual serial port refresh"
```

### 任务 3：WS 报文面板与整体交付

**文件：**
- 修改：`src/assets/assistant.html`
- 修改：`src/assets/js/assistant-page.js`
- 修改：`src/assets/js/format.js`
- 修改：`src/assets/style.css`
- 修改：`tests/admin_assets.rs`
- 修改：`tests/js/assistant-behavior.test.mjs`
- 修改：`README.md`

**DOM 与状态接口：**

新增 id：`assistant-ws-trace`（`<details>`，无 `open` 属性）、`assistant-ws-trace-ts`、`assistant-ws-trace-pause`、`assistant-ws-trace-clear`、`assistant-ws-trace-export`、`assistant-ws-trace-note`、`assistant-ws-trace-log`。

页面状态增加 `traces: []`、`tracePaused: false`、`traceShowTs: true`，模块常量 `TRACE_LIMIT = 2000`。任务 1 entry 到页面后统一变为 `{...entry, ts: Date.now()}`。

- [ ] **步骤 1：编写失败的面板控制点测试**

在 `tests/admin_assets.rs` 增加 `assistant_has_chronological_ws_trace_panel`，对真实 `assistant.html` 断言：

- 存在 `assistant-ws-trace`、`assistant-ws-trace-ts`、`assistant-ws-trace-pause`、`assistant-ws-trace-clear`、`assistant-ws-trace-export`、`assistant-ws-trace-note`、`assistant-ws-trace-log` 这些 id。
- `assistant-ws-trace` 是默认收起的 `<details>`（开始标签没有 `open` 属性），面板含 `/help` 链接与说明文本。

另在 Node 脚本增加 `panel` 用例，并在 `tests/admin_assets.rs` 增加 `assistant_trace_buffer_is_capped` 调用它：对真实 `format.js` 导出的 `pushCapped(items, item, limit)` 连续插入 2001 条（limit=2000），断言长度为 2000、首条是原来的第 2 条、末条是第 2001 条；再用小 limit 验证同一行为。

不要对 JS 源码字符串做断言。`recordTrace` 的时序、暂停/收起、原文 textContent 与导出行为由步骤 8 的真实页面验收；本次不引入 jsdom。

- [ ] **步骤 2：运行测试验证失败**

分别运行：

- `cargo test --test admin_assets assistant_has_chronological_ws_trace_panel -- --exact`
- `cargo test --test admin_assets assistant_trace_buffer_is_capped -- --exact`

预期：前者报告缺少 WS trace DOM id；后者因 `format.js` 尚未导出 `pushCapped` 而失败。

- [ ] **步骤 3：添加默认收起的面板 HTML**

在 `.assistant-main` 后、`.assistant-commands` 前添加 `<details id="assistant-ws-trace" class="card assistant-trace">`，不要写 `open`。包含：

- `<summary>WS 报文</summary>`。
- 说明「观察管理 WS 与数据 WS 的报文原文，可对照帮助文档的通信协议与 close code 表。」并链接 `/help`（新标签、`noopener`）。
- 使用现有 `.assistant-toolbar` 风格的时间戳复选框（默认 checked）、暂停、清空、导出按钮。
- `#assistant-ws-trace-note` 初始 `hidden`，以及 `#assistant-ws-trace-log`。

- [ ] **步骤 4：实现统一记录、上限与渲染**

在 `src/assets/js/assistant-page.js` 中实现以下函数，DOM 文本一律用 `textContent`：

1. 在 `format.js` 导出 `pushCapped(items, item, limit)`：追加 item，并从最旧端裁剪到 `items.length <= limit`，不改变其他元素顺序。`assistant-page.js` 导入它。
2. `recordTrace(entry)`：复制 entry 并附 `ts:Date.now()`，调用 `pushCapped(state.traces, trace, TRACE_LIMIT)` 按到达顺序记录。只有「未暂停且 details 已展开」时追加 DOM，因此收起/暂停不会停止内存捕获。
3. `traceLineElement(trace)`：一行四列——时间戳、方向、操作类型、原文/描述；方向映射 `out→→`、`in→←`、`event→—`。时间戳列按 `state.traceShowTs` 填 `fmtTime(trace.ts)` 或空字符串，kind 与 `trace.text` 均保持完整文本。
4. `renderTrace()`：清空并重绘当前缓冲，空时显示 `暂无 WS 报文`；展开状态下绘制后滚到底部。`updateTraceNote()` 在达到 2000 条时显示 `WS 报文缓冲已满（2000 条），已丢弃最旧数据`，否则隐藏。
5. details 的 `toggle`：展开且未暂停时 `renderTrace()`；收起不清空 state。
6. 暂停按钮：切换 `tracePaused` 和「暂停/继续」文案；继续时重绘。清空按钮只清 `state.traces` 和报文 DOM，不动 `session.frames`。时间戳切换后重绘。
7. 导出：无记录时不下载；有记录时用 `download()` 生成 `ws2s-assistant-ws-${Date.now()}.txt`，每行为 `fmtTime(ts)\t方向\tkind\ttext`。
8. `main()` 在 `client.connect()` 前注册 `client.onTrace(recordTrace)`；`openSerial()` 的 `SerialSession` hooks 增加 `onTrace: recordTrace`。这样 hello、自动对齐和打开串口全过程都会按到达顺序进入同一缓冲。

- [ ] **步骤 5：添加面板样式**

在 `src/assets/style.css` 的 assistant 区域增加 `.assistant-trace` 上间距、`.assistant-trace-log` 深色等宽滚动区域（建议高 320px）和 `.assistant-trace-line` 四列网格。复用现有日志颜色：入站绿色、出站黄色、生命周期事件使用 muted；操作类型列使用等宽字体并允许换行，原文列 `overflow-wrap:anywhere`。窄屏下缩小时间戳/方向列，不改变现有串口日志样式。

- [ ] **步骤 6：更新 README**

在 `README.md` 的「串口助手」说明中加入「WS 报文原文观察（按时间平铺、操作类型、暂停/导出）」和「手动刷新串口」，不更改其他工具页说明。

- [ ] **步骤 7：运行自动测试验证通过**

运行：`cargo test --test admin_assets`

预期：PASS。

再运行：`cargo test`

预期：全部测试 PASS；若有失败，区分本次回归与既有失败，不忽略本次失败。

- [ ] **步骤 8：在真实页面核对协议原文**

启动本机服务并打开 `/assistant`，展开「WS 报文」：

1. 初始应依次看到管理 WS `connect`、`event:hello`、自动 `ports.list`/`session.list` 的 `→` 请求与 `← ... ok` 回应；JSON 为单行原文，回应 action 与请求匹配。
2. 打开一个可用串口：应看到含真实 `path`、`baudRate` 的 `session.open` 原文、回应中的 `sessionId/dataUrl`，以及 `data.connecting`、`data.open`。
3. 发送文本 `ABC`：串口日志仍显示 TX 内容，WS 面板新增 `data.send` 和 `3 B`，不重复完整负载。
4. 关闭串口：应看到 `session.close` 往返及 `data.close`，close code 与实际一致。可做异常断开时，再核对 grace 对应 close code/reason。
5. 收起面板后执行操作再展开，记录仍在；暂停后执行操作，DOM 不追加，继续后记录补齐。清空只影响 WS 面板，导出文件包含时间、方向、操作类型和原文。
6. 在串口框输入 `<img src=x onerror=alert(1)>` 并尝试打开串口：请求失败可以接受，但 WS 面板必须把出站 JSON 中的这段字符串按文本显示；不得弹窗，DevTools 中不得出现对应 `img` 节点。

无可用串口或无法做异常断开时，明确记录未覆盖项，不把静态测试当作手工验收已通过。

- [ ] **步骤 9：Commit**

```bash
git add src/assets/assistant.html src/assets/js/assistant-page.js src/assets/js/format.js src/assets/style.css tests/admin_assets.rs tests/js/assistant-behavior.test.mjs README.md
git commit -m "feat: add assistant websocket message viewer"
```
