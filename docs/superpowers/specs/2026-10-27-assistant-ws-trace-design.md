# 串口助手 WS 报文观察与串口刷新设计

- 日期：2026-10-27
- 状态：已批准（2026-10-27）
- 相关实现：`src/assets/assistant.html`、`src/assets/js/{api,serial,assistant-page,format}.js`、`src/assets/style.css`、`tests/admin_assets.rs`

## 1. 背景与意图

串口助手（`/assistant`）目前只展示**解码后的串口数据**（收发帧、文本/HEX），用户看不到 WebSocket 这一层到底发生了什么：打开串口时管理 WS 上走了什么 JSON、服务端推了什么事件、数据 WS 何时建立/以什么 close code 断开。而帮助文档（`/help`）讲的全是这一层的协议——用户想照着文档自己写客户端时，缺少一个"看真实样例"的窗口。

为此在串口助手页内增加一块 **WS 报文观察面板**：用户每执行一个操作（打开串口、发送、关闭、重连、选历史……），面板按时间顺序平铺显示出对应 WebSocket 报文的原文，每条报文带操作类型标记，可与帮助文档逐条对照。

同时给串口号输入框加一个**刷新串口**按钮：插拔 USB 串口后用户可以手动立即重新扫描，而不是依赖事件同步（事件同步保持不变，按钮是补充）。

## 2. 目标

1. 串口助手页新增「WS 报文」面板：按时间顺序平铺，显示管理与数据两条 WS 通道的关键报文/事件的**原文文本**，每条带操作类型标记。
2. 管理 WS 的出站报文显示请求 JSON 原文（如 `{"id":"c…-1","action":"session.open","params":{…}}`），入站报文显示回应/事件 JSON 原文（成功回应含 `ok/data`，失败回应含 `ok/error`），回应行能还原它对应的是哪个 action。
3. 数据 WS 这层没有 JSON，但仍要记录它与帮助文档 close code 章节对应的形态：连接建立（URL 尾段）、二进制 `ws.send`（字节数）、断开（`close code` + `reason`）。
4. 串口号输入框旁新增「刷新串口」按钮，点击立即重新拉取 `ports.list` 并更新下拉提示。
5. 面板沿用串口日志的工具习惯：时间戳、「暂停/继续」「清空」「导出」；报文行按最近上限截断并提示。
6. 纯前端实现，不新增/修改管理协议动作与事件，不触碰 Rust 后端。

## 3. 非目标

- 不做按操作分组的折叠视图（一次操作的完整往返不归为一组），只按时间平铺。
- 不做报文重放、编辑修改后发送、或"从报文构造请求"。
- 不在管理页、功能测试页、独立演示页复制该面板（如后续需要在别处观察，另行设计）。
- 不做报文侧的结构化高亮/字段树视图；JSON 一律以单行原文显示。
- 不把 WS 报文写盘；导出仅由用户手动触发。
- 不改变串口日志的现有行为与样式语义。

## 4. 反目标

- 面板只展示 `debug.frame` 那类调试帧，用户仍看不到管理 WS 原文。
- 报文原文被"美化"到看不出线上真实形态（必须是 `JSON.stringify` 后的原文文本，不重排字段、不省略字段）。展示格式化/pretty 也不是本设计目标。
- 数据 WS 的串口负载在报文面板里重复完整显示，导致与串口日志大面积重复。
- 刷新串口按钮与现有 `ports.changed` 事件同步互相打架（重复渲染、闪烁或覆盖用户正在输入的串口号）。

## 5. 已确认的决策

来自与用户的对话，建造者不要改动这些决定：

1. **平铺**：报文按时间顺序逐条平铺，不按操作分组（用户 2026-10-27 明确）。
2. **操作类型标记**：每条报文都显示其操作类型（出站请求显示 action 名，如 `session.open`；入站回应显示它对应的 action 名 + ok/error；入站事件显示 `事件:名称`；连接/断开/close 显示生命周期类型）。用户明确要"每个报文有操作类型"。
3. **范围只在串口助手页**：管理页/功能测试页不加该面板与刷新按钮（用户需求专指串口助手）。
4. **每次操作可见对应报文**：凡浏览器实际收发的管理 WS 报文原文都记录，包括页面自动对齐用的 `session.list`/`ports.list`（这正是理解文档的一部分），不过滤自动请求。

## 6. 设计细节

### 6.1 报文来源与捕获点（`api.js`）

- `ManagementClient` 增加原文观察钩子（构造参数或 `onTrace(cb)`，与 `onEvent/onSync/onConnection` 风格一致）。捕获点：
  - 出站：`request(action, params)` 在 `ws.send(JSON.stringify(payload))` 处，捕获**完整 JSON 字符串**与 direction `→`。
  - 入站：`handleMessage(raw)` 在 `JSON.parse` 之前捕获 `raw` 原文与 direction `←`。
  - 连接：`ws.onopen` 记 `connect`（管理 WS），`ws.onclose` 记 `disconnect`。
- 回应行还原 action：`request()` 在 `this.pending` 中保存等待项时，额外保存该请求的 `action`；`handleMessage` 解析出入站回应的 `id` 后，在分发前读取对应等待项的 action。未命中等待项的迟到回应标为 `response`。不得改变现有 `resolveResponse` 对 pending 的清理与超时处理。
- 事件行标 `event:名称`（`hello`、`ports.changed`、`session.state`、`admin.state`）。

### 6.2 报文来源与捕获点（`serial.js` 数据 WS）

- `connectData`：发起连接可记 `data.connecting`（含 dataUrl 尾段）；`ws.onopen` 记 `data.open`。
- `sendBytes`：`this.ws.send(bytes)` 处记 `data.send`，载荷信息为**字节数**（不重复二进制内容；串口日志已完整展示；是否附 ≤32 字节 HEX 预览留建造者决定，默认不加）。
- `ws.onclose`：记 `data.close`，含 `code` 与非空时的 `reason`——这是对照帮助文档 close code 表的关键行。

### 6.3 面板与渲染（`assistant-page.js` / `assistant.html` / `style.css`）

- 新增卡片「WS 报文」，放在 `assistant-main`（串口日志+发送卡片）之后、快捷指令卡片之前，全宽、默认**收起**。可用 `<details><summary>` 或等价实现，具体折叠实现留建造者决定，但默认收起不变。
  - 说明文字示例：「观察管理 WS 与数据 WS 的报文原文，可对照帮助文档的通信协议与 close code 表。」
- 行格式：`时间戳  方向  操作类型  原文/描述`。管理报文的方向为 `→`（出站）或 `←`（入站），生命周期事件的方向显示 `—`。JSON 以单行原文显示；`data.send` 这类无 JSON 的行在原文位置显示 `12 B` 等描述。面板自带时间戳开关，与暂停/清空配套，不依赖串口日志的显示选项。
- 缓冲策略：与串口日志同风格但独立——报文行上限 **2000 行**，超出丢最旧并显示一行提示（同 `assistant-buffer-note` 的样式形态，可复用同款 hint）。
- DOM 增量渲染：`renderAll` / `appendFrame` 之外为报文写平行的 `appendTrace` / `renderTrace`，暂停时继续捕获到内存但不追加 DOM，继续时全量重绘；导出生成 `时间\t方向\t类型\t原文` 的文本。
- 面板收起时也持续捕获报文，展开即可看到此前记录；展开/收起本身不重置数据。

### 6.4 刷新串口按钮

- 位置：串口号输入框附近（建议与 datalist 之间或「打开串口」按钮列旁，建造者决定具体落点）。
- 行为：点击调用 `client.refreshPorts()` → `renderPorts(client.ports)`；管理 WS 未连接时给出与现有失败一致的提示。按钮进行中禁用。
- 与自动同步共存：`ports.changed` 事件、`hello` 对齐路径保持不变；`renderPorts` 现有"未选择时填第一个串口"逻辑不变，不覆盖用户已输入的串口号。

### 6.5 文档与测试

- README `管理页与两个工具页` 段：串口助手描述加一句 WS 报文观察与刷新串口。
- `tests/admin_assets.rs`：现有资产断言是白名单式存在性检查，新增 id（建议 `assistant-refresh-ports`、`assistant-ws-trace-*` 前缀）并补充相应断言。
- 无 JS 测试设施；验收 = `cargo test` 全绿 + 运行时手工核对：打开串口的完整往返（`session.open` 请求/回应/hello/对齐请求）、串口数据 TX `data.send`、拔线/关串口出现 `data.close(1000)` 或 grace 相关 close、刷新串口按钮即时生效。

## 7. 留给建造者决定的部分

- 报文行的方向图标（→/←）与类型标记具体配色/样式。
- 收起/展开的具体实现形态，以及收起时摘要是否显示报文计数。
- 刷新串口按钮精确落点与禁用态文案。
- data.send 是否附 HEX 预览（默认不加）。
- 导出文件名与格式细节（建议沿用 `download()` 与 `ws2s-assistant-ws-${Date.now()}.txt` 风格）。

## 8. 约束

- 零构建、无 CDN、无外部依赖；兼容 Chrome 109 / Firefox 115 ESR。
- 不修改管理协议、close code、会话生命周期行为；本设计只读观察。
- `api.js` 的观察钩子不能改变现有连接/重连/超时行为，`serial.js` 同理（捕获点是旁路观察，失败时静默降级，不得因回调异常中断收发）。
- 现有资产测试是存在性断言，只需追加不需重写。
