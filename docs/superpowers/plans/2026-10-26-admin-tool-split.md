# 管理工具三页面拆分实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 subagent-driven-development（推荐）或 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 把当前单页管理工具拆成 `/` 管理页、`/assistant` 串口助手、`/test` 功能测试三个独立页面；助手可自己打开/关闭串口并完成日常人工收发调试，管理页只做状态监控与帮助，测试页复用现有 7 子 tab 演示。

**架构：** 三个 HTML 文档共用 `style.css`；抽出 `format.js`（纯工具）与 `serial.js`（`SerialSession` 数据 WS 会话），供测试页与助手页复用。每个页面各自创建 `ManagementClient`；管理页用一个客户端做状态监控与 2 秒轮询，助手页用一个管理客户端 + 一个数据 WS，测试页继续用 `createDemo`。`http/mod.rs` 增加 `/assistant`、`/test` 路由和 `Cache-Control: no-store`。

**技术栈：** Rust 1.77.2、axum 0.7（内嵌静态资源）、原生 HTML/CSS/ES2020、无构建步骤、无 CDN。

**规格：** `docs/superpowers/specs/2026-10-26-admin-tool-split-design.md`

## 全局约束

- 零构建、无 CDN、无外部字体；兼容 Chrome 109 / Firefox 115 ESR。
- 所有监听硬编码 `127.0.0.1`；不新增管理协议动作/事件。
- 串口助手每页只管理一个会话；多串口通过多个标签页。
- 串口助手必须通过数据 WS 收发，不得用 `debug.frame` 充当生产数据通道。
- `PORT_BUSY` 不自动重试；必须显示占用指引。
- 文本发送 UTF-8；接收支持 UTF-8/GBK；GBK 发送不在首版范围，提示用户用 HEX。
- 关闭页面沿用 10 秒宽限；助手提供「关闭串口」立即释放。
- 三份 HTML 的 `id` 分别唯一；助手样式限定在 `#assistant-page`，测试页样式继续限定在 `#demo`。
- 独立深色演示页 `demo/websocket2serial-demo.html` 本轮不修改；README 继续写明两份演示的同步义务。
- 每个任务结束前运行 `cargo fmt --check`、`cargo clippy -- -D warnings`、对应测试，并提交。

## 审查重点（Review Focus）

1. **`SerialSession` 抽取不得回归测试页**：A/B 打开、双向 4KB、调试写、宽限重连、一键演示必须与拆分前一致；任务 1 的回归手工检查必须覆盖。
2. **助手必须真正占用并释放串口**：打开后 active、关闭后端口立即释放；`PORT_BUSY` 显示管理页关闭指引且不自动重试；任务 2 手工检查覆盖。
3. **助手收发必须走数据 WS**：高吞吐/连续数据不能依赖 best-effort 的 `debug.frame`；缓冲上限 5000 帧 / 4 MiB，定时发送在断开/关闭时停止。
4. **三个页面互不污染**：路由独立、ID 分别唯一、助手样式不泄漏到管理/测试页；管理页不再包含演示与旧监控面板。
5. **管理页监控仍可用**：会话计数 2 秒刷新、关闭会话、管理服务开关、端口「用助手打开」、帮助文档在拆分后都正常。

## 文件结构

| 文件 | 职责 |
|---|---|
| `src/assets/index.html` | 管理页：入口卡片、服务状态、串口、会话、帮助 |
| `src/assets/assistant.html` | 串口助手页 |
| `src/assets/test.html` | 功能测试页（现有演示 DOM） |
| `src/assets/style.css` | 三页共享基础 + `#demo` + `#assistant-page` |
| `src/assets/js/format.js` | 新建：时间/字节/HEX/Base64/下载等纯函数 |
| `src/assets/js/serial.js` | 新建：`SerialSession` 数据 WS 会话 |
| `src/assets/js/api.js` | 共享 `ManagementClient` |
| `src/assets/js/overview.js` | 管理页：状态、端口、会话、轮询 |
| `src/assets/js/main.js` | 管理页启动 |
| `src/assets/js/assistant-page.js` | 新建：助手页逻辑 |
| `src/assets/js/demo.js` | 功能测试逻辑；改用 `format.js` / `serial.js` |
| `src/assets/js/test-page.js` | 新建：测试页启动 |
| `src/http/mod.rs` | 三页路由、JS 分发、`no-store` |
| `tests/admin_assets.rs` | 三页 DOM/路由/ID/导航/提示/缓存测试 |

---

### 任务 1：抽取 `format.js` 与 `serial.js`，重构 `demo.js`

**文件：**
- 创建：`src/assets/js/format.js`、`src/assets/js/serial.js`
- 修改：`src/assets/js/demo.js`、`src/http/mod.rs`、`tests/admin_assets.rs`

**接口（后续任务使用）：**

```js
// format.js
export const sleep, now, shortId, fmtTime, bytesToHex, bytesToAscii, bytesEqual,
  textEncoder, b64encodeBytes, b64decodeBytes, hexToBytes, pattern, download, stateClass;

// serial.js
export class SerialSession {
  constructor(slot, info, params, client, hooks = {});
  // hooks.onFrame(frame)  frame = { ts, dir: "RX"|"TX", bytes }
  // hooks.onState(session) 连接/关闭/updateInfo 后调用
  // hooks.onLog(level, message) level = "info"|"ok"|"warn"|"error"
  async connectData(timeout = 5000);
  async reconnectOld();
  sendBytes(bytes);
  sendText(text);
  sendHex(text);
  closeData(code = 1000, reason = "");
  disconnectAbnormal(code = 3001);
  waitClose(timeout = 2500);
  async closeSession(asClient = false);
  updateInfo(info);
  takeBytes(n, timeout = 5000);
  clearBuffer();
  // 公开字段：slot, sessionId, dataUrl, params, state, counters,
  // graceEndsAt, lastGap, lastError, frames, frameBytes, closeCode, closeReason, autoReconnect
}
```

- [ ] **步骤 1：编写失败的测试**

在 `tests/admin_assets.rs` 的 `serves_js_assets` 列表加入 `"format.js"`、`"serial.js"`；在 `demo_module_uses_shared_client_contract` 增加：

```rust
assert!(js.contains("from \"./format.js\""), "demo.js 应复用 format.js");
assert!(js.contains("from \"./serial.js\""), "demo.js 应复用 serial.js");
assert!(!js.contains("class SessionController"), "会话控制器应移到 serial.js");
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets -- --nocapture`
预期：`/js/format.js`、`/js/serial.js` 404；`demo.js` 缺少 import。

- [ ] **步骤 3：创建 `format.js`**

从 `demo.js` 迁移纯函数，保持现有行为与返回值不变。至少包含 `sleep`、`now`、`shortId`、`fmtTime`、`bytesToHex`、`bytesToAscii`、`bytesEqual`、`textEncoder`、`b64encodeBytes`、`b64decodeBytes`、`hexToBytes`、`pattern`、`download`、`stateClass`。不要包含 DOM 相关函数。

- [ ] **步骤 4：创建 `serial.js`**

从 `demo.js` 迁移 `SessionController`，改成无 DOM 的 `SerialSession`：

- 删除 `_updateTerminalHead`、`_renderMeta` 与所有 `$("#demo-...")` 查询。
- 状态/连接变化时调用 `hooks.onState(this)`；每帧调用 `hooks.onFrame(frame)`；日志改调 `hooks.onLog(level, message)`。
- 保留帧上限 5000 / 4 MiB、`takeBytes`、`sendBytes`、`sendText`、`sendHex`、`closeData`、`disconnectAbnormal`、`waitClose`、`closeSession`、`reconnectOld`、`updateInfo`、自动重连。
- `closeSession` 继续用 `client.request("session.list"/"session.close")`，返回 data 直接使用。

- [ ] **步骤 5：重构 `demo.js` 使用两个新模块**

- 删除已迁移的工具函数与 `SessionController` 类，改为 `import`。
- `openSlot` 创建：

```js
const controller = new SerialSession(slot, { ...data, state: "awaiting" }, params, client, {
  onFrame: (frame) => appendTerminalLine(slot, frame),
  onState: () => renderTerminalMeta(),
  onLog: (level, message) => logEvent(level, message),
});
```

- `renderTerminalMeta` 直接读取 `controller.state / counters / closeCode / closeReason / graceEndsAt` 更新 `#demo-term-*`，不再调用 `_renderMeta`。
- 把原来的 500ms `_renderMeta` 循环改为 `setInterval(renderTerminalMeta, 500)`，保留 grace 倒计时。
- `data-term` 的 clear/reconnect/abnormal/close 继续通过 `SerialSession` 公开方法与字段完成。

- [ ] **步骤 6：在 `http/mod.rs` 分发新模块**

增加 `JS_FORMAT`、`JS_SERIAL` 与 `/js/*` match 分支。

- [ ] **步骤 7：运行测试与语法检查**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'
Get-ChildItem src\assets\js\*.js | ForEach-Object { Copy-Item $_.FullName $tmp -Force; node --check $tmp; if ($LASTEXITCODE -ne 0) { throw $_.Name } }
```

- [ ] **步骤 8：回归手工检查**

`cargo run` 打开 `http://127.0.0.1:8766/` → 功能演示：打开 COM3/COM4、双向发送、4KB 回环、调试订阅/调试写、异常断开/宽限重连、一键演示（16 pass + 1 skip）。行为必须与拆分前一致。

- [ ] **步骤 9：Commit**

```bash
git add src/assets/js/format.js src/assets/js/serial.js src/assets/js/demo.js src/http/mod.rs tests/admin_assets.rs
git commit -m "refactor: 抽取串口会话与格式化模块"
```

---

### 任务 2：串口助手页核心（连接、收发、日志、导出）

**文件：**
- 创建：`src/assets/assistant.html`、`src/assets/js/assistant-page.js`
- 修改：`src/assets/style.css`、`src/http/mod.rs`、`tests/admin_assets.rs`

**接口：** 页面根 `#assistant-page`；助手页用 `getConfig()` + 一个 `ManagementClient` + `SerialSession`；不从管理页继承状态。

**必需 ID：**

`assistant-page`、`assistant-notice`、`assistant-port`、`assistant-port-list`、`assistant-baud`、`assistant-databits`、`assistant-parity`、`assistant-stopbits`、`assistant-flowcontrol`、`assistant-dtr`、`assistant-rts`、`assistant-open`、`assistant-close`、`assistant-open-result`、`assistant-counters`、`assistant-last-error`、`assistant-state`、`assistant-log`、`assistant-pause`、`assistant-clear`、`assistant-autoscroll`、`assistant-ts`、`assistant-format`、`assistant-export`、`assistant-buffer-note`、`assistant-send-mode`、`assistant-send-eol`、`assistant-send-input`、`assistant-send-btn`、`assistant-send-status`。

- [ ] **步骤 1：编写失败的测试**

在 `admin_assets.rs` 增加：

```rust
#[test]
fn assistant_page_has_notice_and_controls() {
    let html = include_str!("../src/assets/assistant.html");
    for id in ["assistant-page", "assistant-notice", "assistant-open", "assistant-close",
               "assistant-log", "assistant-send-input", "assistant-export"] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    assert!(html.contains("WebSocket2Serial"));
    assert!(html.contains("Web Serial"));
    assert!(html.contains("PORT_BUSY"));
}
```

并把 `assistant-page.js` 加入 `serves_js_assets` 列表；新增 `/assistant` 路由返回 200 的断言。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets -- --nocapture`
预期：`assistant.html` 不存在、`/assistant` 404、`/js/assistant-page.js` 404。

- [ ] **步骤 3：创建 `assistant.html`**

结构：

- 页头：`串口助手`、`#assistant-state`、`← 返回管理页`（`href="/"`）、`功能测试 ↗`（`href="/test"`，`target="_blank"`）。
- 固定提示条 `#assistant-notice`，原文：
  `本页通过 WebSocket2Serial 透传工具访问串口，不是浏览器 Web Serial。同一串口同时只允许一个数据客户端；若提示 PORT_BUSY，请先在管理页关闭占用会话，或关闭其他串口程序。`
- 连接卡片：端口输入 + `datalist`、参数、DTR/RTS、打开/关闭、状态、计数、错误。
- 日志卡片：工具栏（时间戳、HEX/ASCII、暂停、清空、自动滚动、导出）、`#assistant-log`、缓冲提示。
- 发送卡片：文本/HEX、行尾、发送框、发送按钮、发送状态。

`<script type="module" src="/js/assistant-page.js">`。

- [ ] **步骤 4：在 `style.css` 增加 `#assistant-page` 样式**

只使用 `#assistant-page` 作用域；沿用管理页浅色变量。日志区深色底（`#0f1115` / `#d1d5db`），宽度约 1500px，宽屏两列、1050px 以下单列。

- [ ] **步骤 5：实现 `assistant-page.js` 核心**

- `getConfig()` → `new ManagementClient(config.managementPort)` → `client.connect()`。
- 端口：`ports.list` + `ports.changed` → 填 `#assistant-port-list`；从 `new URLSearchParams(location.search).get("port")` 预填。
- 打开：读取参数 → `client.request("session.open", params)` → `new SerialSession("A", data, params, client, hooks)` → `connectData()`；成功后 active。
- 关闭：`session.closeSession(false)` → 清理状态；保留日志。
- 日志：`onFrame` 追加帧；`format.js` 的 HEX/ASCII；时间戳开关；暂停时只缓存不渲染；恢复时重放最后 5000 帧；缓冲满显示 `#assistant-buffer-note`。
- 导出：TXT 行格式 `时间\t方向\tHEX\tASCII`。
- 管理 WS 断开：打开/关闭与会话状态操作禁用，但已连接的数据 WS 仍可继续收发；页头提示离线。
- 数据 WS 连接失败：提示错误并保留关闭会话入口，不自动反复重连。
- `PORT_BUSY`：`#assistant-open-result` 显示「串口已被占用」和「打开管理页关闭会话」链接；不自动重试。

- [ ] **步骤 6：在 `http/mod.rs` 增加 `/assistant` 与 `assistant-page.js`**

`const ASSISTANT_HTML: &str = include_str!("../assets/assistant.html");`，路由 `/assistant`，JS match `assistant-page.js`。

- [ ] **步骤 7：运行测试与语法检查**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\assistant-page.js $tmp -Force; node --check $tmp
```

- [ ] **步骤 8：手工检查**

打开 `http://127.0.0.1:8766/assistant`：选 COM3/115200 打开；向 COM4 发文本与 HEX；确认日志、时间戳、HEX/ASCII、暂停、清空、导出；关闭后端口释放；刷新页面进入 grace 后能重新打开。

- [ ] **步骤 9：Commit**

```bash
git add src/assets/assistant.html src/assets/js/assistant-page.js src/assets/style.css src/http/mod.rs tests/admin_assets.rs
git commit -m "feat: 增加独立串口助手页核心收发"
```

---

### 任务 3：串口助手增强（编码、快捷指令、历史、定时、校验和、统计）

**文件：**
- 修改：`src/assets/assistant.html`、`src/assets/js/assistant-page.js`、`src/assets/style.css`、`tests/admin_assets.rs`

**必需新增 ID：**

`assistant-encoding`、`assistant-history`、`assistant-checksum`、`assistant-timer-interval`、`assistant-timer-count`、`assistant-timer-start`、`assistant-timer-stop`、`assistant-timer-status`、`assistant-cmd-name`、`assistant-cmd-payload`、`assistant-cmd-mode`、`assistant-cmd-eol`、`assistant-cmd-save`、`assistant-cmd-list`、`assistant-rx`、`assistant-tx`、`assistant-rate`、`assistant-drop`。

- [ ] **步骤 1：编写失败的测试**

在 `assistant_page_has_notice_and_controls` 中加入上述 ID 断言。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets assistant_page_has_notice_and_controls -- --nocapture`

- [ ] **步骤 3：补全助手 HTML 控件**

- 接收编码下拉 `#assistant-encoding`（UTF-8 / GBK）。
- 发送历史 `#assistant-history`（select）。
- 校验和 `#assistant-checksum`（none/sum8/xor8/crc16）。
- 定时发送：间隔、次数（0=无限）、开始、停止、状态。
- 快捷指令表单与列表：名称、内容、模式、行尾、保存、列表容器。
- 统计：RX、TX、速率、DROP。

- [ ] **步骤 4：实现增强逻辑**

- `localStorage` 键：
  - `ws2s.assistant.params.v1`
  - `ws2s.assistant.history.v1`（最多 20 条）
  - `ws2s.assistant.commands.v1`
- 快捷指令对象 `{ name, payload, mode, eol }`；列表行提供 `载入`、`发送`、`删除`；所有文本用 `textContent`，不拼 HTML。
- 发送历史：发送成功后 unshift，去重，截断 20；select 变更时回填发送框/模式。
- 校验和：`sum8`、`xor8` 各追加 1 字节；`crc16` 追加 Modbus CRC16 小端 2 字节；只对最终字节序列计算。
- 定时发送：`setInterval`；发送当前发送框内容（含模式/行尾/校验和）；次数到 0 停止；关闭串口、数据 WS 断开、页面隐藏时停止并更新状态。
- 接收编码：`new TextDecoder(label)` 解码；失败回退 UTF-8；发送文本固定 UTF-8，界面提示「GBK/二进制请用 HEX」。
- 统计：RX/TX 取 `SerialSession.counters`；速率用最近 1 秒帧字节数计算，每 500ms 刷新；DROP 显示 `rxDiscardedBytes`。

- [ ] **步骤 5：运行语法检查与测试**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\assistant-page.js $tmp -Force; node --check $tmp
```

- [ ] **步骤 6：手工检查**

保存两条快捷指令并发送；发送历史可回填；定时发送 500ms×5 次后自动停；Sum8/XOR8/CRC16 附加字节可用 HEX 观察；GBK 接收显示中文；统计与速率刷新；关闭串口后定时发送停止。

- [ ] **步骤 7：Commit**

```bash
git add src/assets/assistant.html src/assets/js/assistant-page.js src/assets/style.css tests/admin_assets.rs
git commit -m "feat: 完善串口助手快捷指令、定时发送与校验和"
```

---

### 任务 4：管理页拆分与功能测试独立页

**文件：**
- 修改：`src/assets/index.html`、`src/assets/style.css`、`src/assets/js/main.js`、`src/assets/js/overview.js`、`src/http/mod.rs`、`tests/admin_assets.rs`
- 创建：`src/assets/test.html`、`src/assets/js/test-page.js`
- 删除：`src/assets/js/assistant.js`

- [ ] **步骤 1：编写失败的测试**

- `/test` 路由 200；`test.html` 含 `demo-tab-terminal`、`demo-btn-run-demo`、7 个 `data-demo-tab`。
- `/js/test-page.js` 分发成功；`/js/assistant.js` 返回 404。
- `index.html` 含 `href="/assistant"`、`href="/test"` 且两者带 `target="_blank"`；不再含 `data-main-tab` 或 `id="demo"`。
- 现有 `admin_assets_have_required_controls_and_no_cdn` 中属于旧 `assistant` / `demo` 的 ID 断言迁到 `test.html` / `assistant.html` 的测试，`index.html` 只保留管理页 ID。

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets -- --nocapture`

- [ ] **步骤 3：创建 `test.html` 与 `test-page.js`**

- 把 `index.html` 中 `<section id="demo" ...>` 整段（状态条、7 个子 tab、面板）移到 `test.html`。
- 去掉根节点的 `tab-panel` class（避免被全局 `.tab-panel { display:none }` 隐藏），保留 `id="demo"`。
- 页头：`功能测试`、`← 返回管理页`、`串口助手 ↗`。
- 在 `style.css` 增加三页共用的 `.page-header` / 页头导航样式；删除旧监控面板的 CSS（`.assistant-toolbar`、`.buffer-note`、`.log`、`.frame*`、`.gap-note`、`.send-panel`、`.send-row`、`.write-hint`、`.send-actions`、`.status`），确认测试页 `#demo` 的 `.log-line`、`.event-line`、`.badge` 等样式不受影响。
- `test-page.js`：

```js
import { getConfig, ManagementClient } from "./api.js";
import { createDemo } from "./demo.js";

async function main() {
  const config = await getConfig();
  const client = new ManagementClient(config.managementPort);
  createDemo(document.getElementById("demo"), client);
  client.connect();
}

main().catch((err) => console.error(err));
```

- [ ] **步骤 4：删除旧监控面板**

- `index.html` 删除 `#assistant` 整段、主 tab 导航、`#demo` 整段。
- `main.js` 删除 `createAssistant`/`createDemo` 与 `setupMainTabs`，只保留一个 `ManagementClient` + `bindOverview` + `connect`。
- 删除 `src/assets/js/assistant.js` 与 `http/mod.rs` 的 `JS_ASSISTANT`/`assistant.js` 分支。

- [ ] **步骤 5：管理页保留监控骨架**

`index.html` 暂时保留 `#overview`（服务状态/端口/会话），入口卡片与帮助在任务 5 补；页头放 `/assistant`、`/test` 新标签链接。

- [ ] **步骤 6：运行测试与语法检查**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'
Get-ChildItem src\assets\js\*.js | ForEach-Object { Copy-Item $_.FullName $tmp -Force; node --check $tmp; if ($LASTEXITCODE -ne 0) { throw $_.Name } }
```

- [ ] **步骤 7：手工检查**

管理页两个入口新标签打开 `/assistant`、`/test`；测试页 7 子 tab 与一键演示正常；助手页不受影响；旧 `/js/assistant.js` 404。

- [ ] **步骤 8：Commit**

```bash
git add src/assets/index.html src/assets/test.html src/assets/js/main.js src/assets/js/test-page.js src/assets/js/overview.js src/http/mod.rs tests/admin_assets.rs
git rm src/assets/js/assistant.js
git commit -m "feat: 拆分管理页与功能测试独立页"
```

---

### 任务 5：管理页状态监控增强与帮助文档

**文件：**
- 修改：`src/assets/index.html`、`src/assets/js/overview.js`、`src/assets/style.css`、`tests/admin_assets.rs`

**必需 ID/文本：** `help`、`autostart`、`entry-assistant`、`entry-test`；帮助文本包含 `PORT_BUSY`、`10 秒`、`Web Serial`。

- [ ] **步骤 1：编写失败的测试**

在 `admin_assets.rs` 增加：`index.html` 含 `id="help"`、`id="autostart"`、`id="entry-assistant"`、`id="entry-test"`，且包含 `PORT_BUSY`、`10 秒`、`Web Serial`；`overview.js` 含 `/assistant?port=`。

- [ ] **步骤 2：运行测试验证失败**

- [ ] **步骤 3：补全入口卡片与帮助文档**

- 入口卡片两张：`#entry-assistant` / `#entry-test`，`target="_blank" rel="noopener"`。
- 服务状态增加 `#autostart`（只读；来自 `client.hello.autostart` 或 `state.config.autostart`）。
- 帮助 `#help` 用 `<details>` 分节：快速开始、三个页面、串口参数、串口占用与宽限、常见错误码、浏览器兼容、与 Web Serial 的区别、管理服务关闭后的恢复。
- 帮助文字必须说明：同一串口只能一个数据客户端；`PORT_BUSY` 时去管理页会话表关闭；关闭页面有 10 秒宽限；数据不落盘。

- [ ] **步骤 4：管理页监控增强**

- `overview.js` 端口表每行增加「用助手打开」链接：

```js
const link = document.createElement("a");
link.href = `/assistant?port=${encodeURIComponent(p.path)}`;
link.target = "_blank";
link.rel = "noopener";
link.textContent = "用助手打开";
```

- 会话表增加 DROP 列（`counters.rxDiscardedBytes`）。
- `bindOverview` 启动 2 秒轮询：`setInterval(() => { if (!document.hidden) client.refreshSessions(); }, 2000)`；不要重复注册。
- 保留关闭会话、管理服务开关、复制管理地址。

- [ ] **步骤 5：运行测试与手工检查**

```powershell
cargo test --test admin_assets -- --nocapture
```

手工：管理页会话计数 2 秒变化；端口「用助手打开」预填端口；关闭会话后助手数据 WS 显示 4004；帮助文档可展开；管理服务开关仍带确认。

- [ ] **步骤 6：Commit**

```bash
git add src/assets/index.html src/assets/js/overview.js src/assets/style.css tests/admin_assets.rs
git commit -m "feat: 管理页状态监控与帮助文档"
```

---

### 任务 6：缓存头、路由与静态资源测试收口

**文件：**
- 修改：`src/http/mod.rs`、`tests/admin_assets.rs`

- [ ] **步骤 1：编写失败的测试**

新增异步测试：GET `/`、`/assistant`、`/test`、`/style.css`、`/js/api.js`、`/api/config`，断言 `cache-control` 包含 `no-store`。同时把 `serves_js_assets` 的最终列表固定为：`api.js`、`overview.js`、`main.js`、`demo.js`、`format.js`、`serial.js`、`assistant-page.js`、`test-page.js`；断言 `/js/assistant.js` 404。

- [ ] **步骤 2：运行测试验证失败**

- [ ] **步骤 3：实现 `Cache-Control: no-store`**

在 `index`、`style`、`js`、`assistant`、`test`、`config` 响应头中加入 `(header::CACHE_CONTROL, "no-store")`；可用一个返回 `(HeaderMap, &'static str)` 的小助手统一。

- [ ] **步骤 4：运行测试**

```powershell
cargo test --test admin_assets -- --nocapture
```

- [ ] **步骤 5：手工检查响应头**

```powershell
curl.exe -sI http://127.0.0.1:8766/assistant
curl.exe -sI http://127.0.0.1:8766/js/serial.js
```

预期均含 `cache-control: no-store`。

- [ ] **步骤 6：Commit**

```bash
git add src/http/mod.rs tests/admin_assets.rs
git commit -m "fix: 管理页静态资源禁用浏览器缓存"
```

---

### 任务 7：文档更新

**文件：** `README.md`、`demo/README.md`、`docs/deployment.md`

- [ ] **步骤 1：更新根 README**

说明三个页面与地址：`http://127.0.0.1:8766/`、`/assistant`、`/test`；串口助手能力与占用提示；功能测试复用原演示；独立 `demo/` 页仍保留。

- [ ] **步骤 2：更新 `demo/README.md`**

集成版入口从管理页「功能演示」tab 改为 `/test`；管理页不再有演示 tab；独立深色页仍保留，两份功能需同步。

- [ ] **步骤 3：更新 `docs/deployment.md`**

在验收/使用章节补充三个页面 URL 和“同一串口只能一个数据客户端”的现场说明。

- [ ] **步骤 4：Commit**

```bash
git add README.md demo/README.md docs/deployment.md
git commit -m "docs: 说明管理页、串口助手与功能测试三页入口"
```

---

### 任务 8：最终验证

- [ ] **步骤 1：格式、Lint、全量测试**

```powershell
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

- [ ] **步骤 2：JS 模块语法检查**

```powershell
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'
Get-ChildItem src\assets\js\*.js | ForEach-Object { Copy-Item $_.FullName $tmp -Force; node --check $tmp; if ($LASTEXITCODE -ne 0) { throw $_.Name } }
```

- [ ] **步骤 3：浏览器冒烟**

1. 管理页打开 `/assistant`、`/test` 新标签；两页可返回管理页。
2. 助手页 COM3 打开、COM4 收发、文本/HEX、时间戳、暂停/清空/导出。
3. 快捷指令保存/载入/发送/删除；定时发送 500ms×5 自动停；Sum8/XOR8/CRC16 字节正确。
4. 关闭串口立即释放；重新打开正常；刷新后 grace 复用。
5. 测试页占用 COM3 后助手再开 COM3 → `PORT_BUSY` 指引；管理页关闭会话后助手可开。
6. 管理页会话计数 2 秒刷新；关闭会话后助手显示 4004；端口「用助手打开」预填。
7. 测试页 7 子 tab 与一键演示保持 16 pass + 1 skip。
8. 1050px 以下助手单列、测试页双端口折叠；无横向溢出。
9. Chrome 109 / Firefox 115 ESR 基本收发与切换；控制台无未捕获错误。

- [ ] **步骤 4：修复发现的问题并补提交**

```bash
git add -A
git commit -m "fix: 三页面拆分最终验证修复"
```
