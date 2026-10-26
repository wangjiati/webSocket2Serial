# 管理页集成功能演示（Tab 化）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 subagent-driven-development（推荐）或 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 把独立演示页完整搬进管理页，做成「概览 / 串口助手 / 功能演示」三个主 tab，演示内部保留 7 个子 tab，统一浅色主题，同时保留原独立深色演示页。

**架构：** 管理页仍是零构建的 `index.html` + `style.css` + ES modules。`main.js` 创建两个 `ManagementClient` 实例：一个给概览/串口助手，一个给演示模块；两条管理 WS 连接让双方的 `debug.subscribe` 互不覆盖。演示 HTML 并入 `index.html`（ID 统一加 `demo-` 前缀），样式全部限定在 `#demo` 作用域，逻辑迁入新的 `src/assets/js/demo.js`，由 `createDemo(root, client)` 初始化。

**技术栈：** Rust 1.77.2、axum 0.7（内嵌静态资源）、原生 HTML/CSS/ES2020、无构建步骤、无 CDN。

**规格：** `docs/superpowers/specs/2026-10-26-admin-demo-tabs-design.md`

## 全局约束

- 管理页为纯 HTML/CSS/ES2020，无构建步骤、无外部 CDN、无外部字体；兼容 Chrome 109 与 Firefox 115 ESR。
- 所有新增静态资源通过 `include_str!` 内嵌，由 `src/http/mod.rs` 分发；只允许 `127.0.0.1`。
- 管理页现有 `overview`、`assistant` 两个区域的 DOM 控制点（`ports`、`sessions`、`admin-toggle`、`export` 等）必须保留，现有测试不能删改语义。
- 演示元素 ID 全部以 `demo-` 开头；`index.html` 内所有 `id` 必须唯一。
- 演示 CSS 必须限定在 `#demo` 作用域，不得新增影响概览/串口助手的全局选择器。
- `main` 最大宽度约 1500px，双端口终端在 >1050px 时保持两列，窄屏沿用现有断点折叠。
- `managementPort` 只从 `/api/config` 读取；集成版不提供端口输入框、连接按钮、打开管理页链接或管理页开关。
- 演示模块拥有独立 `ManagementClient` 实例；不得复用管理页客户端的调试订阅。
- `demo/websocket2serial-demo.html` 本轮不修改；两份演示功能变更必须同步，并在 README 中写明。
- 每个任务结束前提交；提交信息用中文 Conventional Commits（如 `feat: ...`）。

## 审查重点（Review Focus）

1. **错误码语义迁移**：`api.js` 的 `request()` 直接返回 `data`，错误对象只有 `.code` / `.message`，原演示用 `e.message.includes("PORT_BUSY")`、`r.data` 的写法必须全部改掉；漏改会让 PORT_BUSY 重试、`session.open`、`session.write`、边界自检静默失效。任务 3、4 的实现步骤与手工冒烟必须覆盖。
2. **切换 tab 不丢状态**：主 tab / 子 tab 反复切换后，已打开的数据 WS、终端日志、调试订阅、事件日志必须仍在；任务 2、3、5 的手工检查要专门切走再切回。
3. **ID 与样式隔离**：演示 ID 重复或 CSS 选择器泄漏会直接改坏概览/串口助手；任务 2、3 的 HTML ID 唯一性测试和视觉检查要覆盖。
4. **两套调试订阅互不覆盖**：串口助手订阅某会话时，演示调试订阅 `all` 或反过来，双方 replay 与帧流都必须各自正常；任务 4 手工检查要同时开两个订阅。
5. **一键演示不得关闭当前管理页**：集成版 `admin` 步骤必须是 skip，失败清理不得调用 `admin.set`，也不得遗留 A/B 会话占住串口；任务 5 手工检查覆盖失败路径。

## 文件结构

| 文件 | 职责 |
|---|---|
| `src/assets/index.html` | 页面骨架：header、主 tab、概览/串口助手/功能演示三个面板、演示状态条、7 个子 tab 与全部演示 DOM |
| `src/assets/style.css` | 现有管理页样式 + 主/子 tab + `#demo` 作用域下的浅色演示样式与深色日志区 |
| `src/assets/js/api.js` | 管理 WS 客户端：新增 `whenReady()`、`unsubscribeDebug()` |
| `src/assets/js/main.js` | 启动：读配置、创建两个客户端、初始化主 tab、绑定三个视图 |
| `src/assets/js/demo.js` | 新建：演示模块，导出 `createDemo(root, client)`，包含端口/会话/终端/调试/边界/一键演示/事件 |
| `src/http/mod.rs` | 内嵌并分发 `/js/demo.js` |
| `tests/admin_assets.rs` | 静态资源、必需控制点、ID 唯一性、`demo.js` 分发测试 |
| `README.md` | 管理页功能演示入口说明；独立页保留与同步义务 |
| `demo/README.md` | 两份演示的关系、差异与同步说明 |

---

### 任务 1：主 tab 外壳、demo.js 骨架与静态分发

**文件：**
- 修改：`src/assets/index.html`、`src/assets/style.css`、`src/assets/js/api.js`、`src/assets/js/main.js`、`src/http/mod.rs`
- 创建：`src/assets/js/demo.js`
- 测试：`tests/admin_assets.rs`

**接口（后续任务使用）：**
- `demo.js` 导出 `createDemo(root, client)`，返回 `{ onShow() }`。
- `api.js` 新增 `whenReady(timeoutMs = 6000)` 与 `unsubscribeDebug()`。
- 主 tab 按钮用 `data-main-tab="overview|assistant|demo"`，面板 ID 为 `overview`、`assistant`、`demo`。

- [ ] **步骤 1：编写失败的测试**

在 `tests/admin_assets.rs` 的必需 ID 列表中加入 `"main-tabs"`、`"demo"`；在 `serves_js_assets` 的路径列表中加入 `"demo.js"`。

```rust
for id in ["overview", "assistant", "main-tabs", "demo", "ports", "sessions", "admin-toggle", "export"] {
    assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets -- --nocapture`
预期：`admin_assets_have_required_controls_and_no_cdn` 报 missing `main-tabs`；`serves_js_assets` 对 `/js/demo.js` 返回非成功。

- [ ] **步骤 3：创建 `src/assets/js/demo.js` 骨架**

```js
// 管理页「功能演示」模块；与 demo/websocket2serial-demo.html 保持功能同步。
export function createDemo(root, client) {
  return { onShow() {} };
}
```

- [ ] **步骤 4：在 `src/http/mod.rs` 分发 `demo.js`**

增加 `const JS_DEMO: &str = include_str!("../assets/js/demo.js");`，并在 `js()` 的 `match` 中增加 `"demo.js" => ("text/javascript; charset=utf-8", JS_DEMO),`。

- [ ] **步骤 5：改造 `index.html` 为三个主 tab 面板**

保留 `header` 和现有两个 `<section>` 内容，把 `<main>` 改为：

```html
<nav id="main-tabs" class="main-tabs" role="tablist" aria-label="管理页">
  <button type="button" role="tab" data-main-tab="overview" aria-selected="true" aria-controls="overview">概览</button>
  <button type="button" role="tab" data-main-tab="assistant" aria-selected="false" aria-controls="assistant">串口助手</button>
  <button type="button" role="tab" data-main-tab="demo" aria-selected="false" aria-controls="demo">功能演示</button>
</nav>
<main>
  <section id="overview" class="tab-panel active" role="tabpanel">…现有内容…</section>
  <section id="assistant" class="tab-panel" role="tabpanel">…现有内容…</section>
  <section id="demo" class="tab-panel" role="tabpanel">
    <p class="muted">演示模块加载中…</p>
  </section>
</main>
```

每个 `section` 增加 `role="tabpanel"`；非首个面板默认隐藏。

- [ ] **步骤 6：在 `style.css` 增加主 tab 与面板基础样式**

需要 `.main-tabs`（横向、可换行、下边框）、`.main-tabs button`、`.main-tabs button.active`、`.tab-panel { display: none; }`、`.tab-panel.active { display: block; }`。不要改动现有 `.badge`、`table`、`button` 规则。

- [ ] **步骤 7：在 `api.js` 增加两个方法**

签名与行为：

```js
whenReady(timeoutMs = 6000) // hello 已到则立即 resolve(hello)；否则等待 hello，超时 reject(new Error("等待管理 WS 连接超时"))
unsubscribeDebug()           // 发送 debug.subscribe {sessionIds: [], replay: false}，清空 this.subscription
```

`whenReady` 需要在 `dispatchEvent` 收到 `hello` 时唤醒等待者；关闭/超时要清理定时器，避免泄漏。

- [ ] **步骤 8：在 `main.js` 创建两个客户端并接线**

```js
const adminClient = new ManagementClient(config.managementPort);
const demoClient = new ManagementClient(config.managementPort);
const demoView = createDemo(document.getElementById("demo"), demoClient);
setupMainTabs(demoView);
adminClient.connect();
demoClient.connect();
```

在 `main.js` 顶部加入 `import { createDemo } from "./demo.js";`。`setupMainTabs` 实现主 tab 点击与方向键切换：同步 `.active`、`aria-selected`，进入 `demo` 时调用 `demoView.onShow()`。概览/串口助手继续绑定 `adminClient`。

- [ ] **步骤 9：运行测试与模块语法检查**

运行：

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'
Get-ChildItem src\assets\js\*.js | ForEach-Object { Copy-Item $_.FullName $tmp -Force; node --check $tmp; if ($LASTEXITCODE -ne 0) { throw $_.Name } }
```

预期：测试 PASS；所有模块 `node --check` 通过。

- [ ] **步骤 10：Commit**

```bash
git add src/assets/index.html src/assets/style.css src/assets/js/api.js src/assets/js/main.js src/assets/js/demo.js src/http/mod.rs tests/admin_assets.rs
git commit -m "feat: 增加主 tab 外壳与演示模块骨架"
```

---

### 任务 2：演示静态结构、浅色样式与子 tab 切换

**文件：**
- 修改：`src/assets/index.html`、`src/assets/style.css`、`src/assets/js/demo.js`
- 测试：`tests/admin_assets.rs`

**接口：** 子 tab 按钮用 `data-demo-tab="overview|ports|sessions|terminal|debug|edge|events"`，面板 ID 为 `demo-tab-<name>`；`demo.js` 的 `createDemo` 负责子 tab 切换与 `onShow()`。

- [ ] **步骤 1：编写失败的测试**

在 `admin_assets_have_required_controls_and_no_cdn` 中加入：

```rust
for id in [
    "demo-statusbar", "demo-tabs",
    "demo-tab-overview", "demo-tab-ports", "demo-tab-sessions", "demo-tab-terminal",
    "demo-tab-debug", "demo-tab-edge", "demo-tab-events",
    "demo-btn-run-demo", "demo-ports-body", "demo-sessions-body",
    "demo-log-A", "demo-log-B", "demo-debug-body", "demo-edge-log", "demo-event-log",
] {
    assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
}
```

同时新增 ID 唯一性测试：

```rust
fn collect_ids(html: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("id=\"") {
        let start = pos + 4;
        let end = start + rest[start..].find('"').unwrap();
        ids.push(rest[start..end].to_string());
        rest = &rest[end..];
    }
    ids
}

#[test]
fn admin_html_ids_are_unique() {
    let html = include_str!("../src/assets/index.html");
    let mut ids = collect_ids(html);
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "index.html 存在重复 id");
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets -- --nocapture`
预期：缺失演示控制点。

- [ ] **步骤 3：迁移演示 HTML 到 `#demo` 面板**

把 `demo/websocket2serial-demo.html` 的 7 个 `<section>` 搬进 `index.html` 的 `#demo`，并做以下调整：

- 每个 ID 加 `demo-` 前缀；子 tab 面板 ID 改为 `demo-tab-overview` … `demo-tab-events`。
- 顶部加状态条 `<div id="demo-statusbar">`，内含 `demo-pill-mgmt`、`demo-pill-version`、`demo-pill-admin`、`demo-pill-autostart`；加 `<div id="demo-banner" class="demo-banner hidden"></div>`。
- 加子 tab 导航 `<nav id="demo-tabs" class="demo-tabs" role="tablist">`，7 个按钮带 `data-demo-tab`。
- 删除原顶栏（管理端口输入、连接/重连、打开管理页、切换管理页）与 footer。
- 删除概览里的「打开管理页」「切换管理页」按钮；「刷新端口/会话」保留并改 ID。
- 概览底部文案去掉 `python -m http.server` 与 `file://` 说明，保留浏览器 close code、虚拟串口 4KB、托盘/自启只读说明。
- 保留 `data-term`、`data-edge`、`data-slot` 等数据属性。

- [ ] **步骤 4：迁移演示样式到 `#demo` 作用域**

从演示页 `<style>` 中迁移除 `:root`、`html,body`、`.topbar`、`.brand`、`.top-actions`、原 `.tabs`、footer 之外的规则。每条选择器前加 `#demo `（例如 `#demo .card`、`#demo .btn`、`#demo table`），并把调色板改为管理页变量：

- 页面/卡片：`var(--bg)`、`var(--panel)`、`var(--border)`、`var(--text)`、`var(--muted)`、`var(--accent)`。
- 终端/日志/事件区：保留深色底 `#0f1115`、文字 `#d1d5db`，与串口助手日志一致。
- 状态徽标：成功 `var(--ok)`、警告 `var(--warn)`、错误 `var(--danger)`。
- 增加 `.demo-tabs`、`.demo-panel { display: none; }`、`.demo-panel.active { display: block; }`、`.demo-statusbar`、`.demo-banner` 的浅色样式，并加 `#demo .hidden { display: none !important; }`（演示 HTML 继续用 `hidden` 类控制空态与 banner）。
- 把 `main` 最大宽度从 1100px 调到约 1500px；确认 1050px / 650px 两个断点下双端口终端、表单网格和演示表格仍正常折叠，不出现横向溢出。

- [ ] **步骤 5：在 `demo.js` 实现子 tab 切换**

`createDemo(root, client)` 内用 `root.querySelector` / `root.querySelectorAll` 绑定 `[data-demo-tab]`，切换 `.active` 与 `aria-selected`；`onShow()` 只做当前面板的重绘入口（先为空实现）。不要使用全局 `document.querySelector(".tabs")`，避免和主 tab 冲突。

- [ ] **步骤 6：运行测试与语法检查**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\demo.js $tmp -Force; node --check $tmp
```

预期：PASS。

- [ ] **步骤 7：手工检查**

`cargo run` 后打开 `http://127.0.0.1:8766/`：主 tab 可切换；功能演示显示浅色状态条与 7 个子 tab；逐个点子 tab 能切换；概览与串口助手外观无变化；控制台无错误。

- [ ] **步骤 8：Commit**

```bash
git add src/assets/index.html src/assets/style.css src/assets/js/demo.js tests/admin_assets.rs
git commit -m "feat: 迁移功能演示结构与浅色样式"
```

---

### 任务 3：演示核心——端口、会话、双端口终端

**文件：**
- 修改：`src/assets/js/demo.js`
- 测试：`tests/admin_assets.rs`

**接口：** 使用传入的 `client`（`ManagementClient`）。**所有 `client.request()` 直接返回 `data`；错误用 `e.code` 判断。** 演示模块不再定义 `class ManagementClient`。

- [ ] **步骤 1：编写静态约束测试**

在 `admin_assets.rs` 增加：

```rust
#[test]
fn demo_module_uses_shared_client_contract() {
    let js = include_str!("../src/assets/js/demo.js");
    assert!(js.contains("function renderTerminal"), "缺少终端渲染实现");
    assert!(js.contains("function openSlot"), "缺少会话打开实现");
    assert!(!js.contains("class ManagementClient"), "演示模块不得自带管理客户端");
    assert!(!js.contains("r.data"), "request() 已直接返回 data");
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test --test admin_assets demo_module_uses_shared_client_contract -- --nocapture`
预期：FAIL，报「缺少终端渲染实现」（骨架还没有 `renderTerminal`）。

- [ ] **步骤 3：迁移工具函数、状态与日志**

把演示脚本中的 `$`/`$$`（改为 root 作用域）、时间/字节/Base64/HEX/pattern/download/toast、`BROWSER_BLOCKED_PORTS`、`state`、`logEvent`、`renderEvents`、`edgeLog` 迁入 `createDemo` 闭包。`toast` 使用 `#demo-banner`，并把 `className` 固定为 `demo-banner`（加提示类型时用 `demo-banner ok/err`）；事件日志使用 `#demo-event-log`。

- [ ] **步骤 4：接入客户端事件与状态对齐**

- `client.onConnection`：更新 `#demo-pill-mgmt`（连接中/已连接/已断开）。
- `client.onEvent`：处理 `hello`（版本、协议、OS、管理页、自启、会话数）、`session.state`、`ports.changed`、`admin.state`、`debug.frame`。
- `client.onSync`：把 `client.sessions` / `client.ports` 同步到演示本地 `state.sessions` / `state.ports`，重绘端口、会话、终端元信息、调试选项。
- 删除原 `ManagementClient` 类、`ensureMgmt`、`alignAfterHello` 与 `renderHelloSessions` 中的重复对齐逻辑；需要等待连接时用 `await client.whenReady()`。重连后的会话/端口对齐与调试订阅恢复由 `api.js` 负责，不要再发第二次 `debug.subscribe`。
- 可以在模块内保留 `const mgmt = client` 作为局部别名减少机械替换，但不得再创建第二个实例。
- 保留 1.5s 轮询计数：`if (client.connected) client.refreshSessions()`。

- [ ] **步骤 5：迁移端口与会话**

- 端口页：`refreshPorts()` 调 `client.refreshPorts()` 后取 `client.ports`；渲染表格、datalist、「设为 A/B」。
- 会话页：`readOpenParams`、`openSlot`、`closeSlot`、`refreshSessions`、`renderSessions`。
- **返回值适配**：`session.open` 的 `r` 直接是 `{sessionId, dataUrl, state, counters, reused}`；`session.list` 的 `r` 直接是 `{sessions}`；`session.close` 忽略返回。
- **错误适配**：`String(e.message).includes("PORT_BUSY")` 改为 `e.code === "PORT_BUSY"`；边界日志显示 `${e.code || "ERROR"}: ${e.message}`。
- 账号/参数（波特率、数据位、校验、停止位、流控、DTR/RTS）沿用现有表单与校验。

- [ ] **步骤 6：迁移数据会话与双端口终端**

迁移 `SessionController`（打开/关闭/异常断开/宽限重连/自动重连/缓冲/计数/日志帧），以及 `renderTerminal`、`renderTerminalMeta`、`renderTerminalLog`、`appendTerminalLine`、`bindTerminalPrefs`、`sendFromSlot`、`loopback`、`exportTerminal`。保持 `dataUrl` 原样连接，不拼管理端口。

- [ ] **步骤 7：运行语法检查与测试**

```powershell
cargo test --test admin_assets -- --nocapture
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\demo.js $tmp -Force; node --check $tmp
```

预期：PASS。

- [ ] **步骤 8：手工检查**

打开功能演示 → 端口页扫描；会话页选参数打开 A/B；双端口终端双向发送、HEX/文本、暂停/清空/导出；关闭 A/B 后端口释放；切到概览再切回，终端日志仍在。若本机没有互连串口对，至少用单个可用串口验证打开/关闭/日志，回环标为环境依赖。

- [ ] **步骤 9：Commit**

```bash
git add src/assets/js/demo.js tests/admin_assets.rs
git commit -m "feat: 迁移演示端口、会话与双端口终端"
```

---

### 任务 4：调试、事件与边界自检

**文件：**
- 修改：`src/assets/js/demo.js`

- [ ] **步骤 1：迁移调试订阅**

迁移 `renderDebugOptions`、`renderDebugState`、`debugFrameRow`、`renderDebugFrames`、`onDebugFrame`、`subscribeDebug`、`unsubscribeDebug`、`exportDebug`、`debugWrite`、`renderDebugWriteOptions`。

- `client.subscribeDebug(ids, replay)` 使用 `api.js` 现有签名；`unsubscribeDebug()` 使用任务 1 新增方法。
- `session.write` 返回值直接是 `{written}`，不要写 `r.data.written`。
- 保持 seq 缺口统计、replay 样式、帧表上限与导出格式。

- [ ] **步骤 2：迁移事件日志**

保留 `renderEvents`、过滤、清空、导出 JSON；接收 `hello`、`ports.changed`、`session.state`、`debug.frame`、`admin.state` 时写入日志。日志上限仍为 1500 条。

- [ ] **步骤 3：迁移边界自检**

迁移 10 个 `data-edge` 处理函数。`edgeWriteInactive` 的 `session.open` 结果直接取 `sessionId`；错误提示使用 `e.code`。保留确认弹窗（文本帧/超大帧）与「临时借用 A 端口」逻辑。

- [ ] **步骤 4：运行语法检查**

```powershell
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\demo.js $tmp -Force; node --check $tmp
```

- [ ] **步骤 5：手工检查**

打开 A 后：调试页订阅 `all`，在终端发数据，看到 `debug.frame` 与 seq；调试写能让 B 收到；串口助手同时订阅一个会话，双方互不覆盖且 replay 都正常；边界页逐个执行 4002 / PORT_BUSY / PORT_NOT_FOUND / SESSION_STATE / UNKNOWN_ACTION / 文本帧 1003 / 超大帧 1009 / 异常断开 grace / 正常关闭，结果写入边界日志且页面不崩。

- [ ] **步骤 6：Commit**

```bash
git add src/assets/js/demo.js
git commit -m "feat: 迁移演示调试、事件与边界自检"
```

---

### 任务 5：一键完整演示与演示概览收尾

**文件：**
- 修改：`src/assets/js/demo.js`

- [ ] **步骤 1：迁移一键演示**

迁移 `runnerSteps`、`renderRunner`、`runnerSet`、`waitSessionState`、`runDemo`。适配点：

- 第一步 `connect` 改为 `await client.whenReady()`，详情显示 `127.0.0.1:${client.managementPort}`。
- `session.write` 结果取 `w.written`。
- `state.mgmt.subscribeDebug` 改为 `client.subscribeDebug`；清理时 `client.unsubscribeDebug()`。
- `admin` 步骤直接标 `skip`，详情「集成在管理页内，跳过关闭自身；可在概览页手动验证」。
- 清理阶段删除 `admin.set {enabled:true}`；仍要关闭 A/B、退订调试、刷新会话。
- 失败时保持现有 fail/skip 标记与 `cleanup` 步骤。

- [ ] **步骤 2：收尾演示概览**

- `onHello` 更新连接信息与状态条；`renderAdmin` 只更新 `demo-pill-admin` / `demo-pill-autostart` / `demo-hello-admin`，不再操作链接与管理页开关。
- 删除 `toggleAdmin`、`#link-admin`、`#btn-admin-toggle` 相关代码。
- `renderOrigin` 去掉 `file://` 与 `python -m http.server` 文案；非 loopback 时仅在事件日志提示。
- 「快速开始」改为管理页内的实际操作顺序：端口页确认串口 → 会话页打开 A/B → 双端口终端验证 → 一键演示。

- [ ] **步骤 3：运行语法检查与全量测试**

```powershell
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'; Copy-Item src\assets\js\demo.js $tmp -Force; node --check $tmp
cargo test
```

预期：全部 PASS。

- [ ] **步骤 4：手工检查**

有互连串口对时跑完整一键演示：`admin` 步骤显示 skip，其余步骤通过或给出「串口未互连」的明确提示；结束后 A/B 已关闭、无残留会话。故意让 A/B 选同一个端口或选不存在的端口，确认失败被标记且清理仍执行；管理页始终可访问、不需要重新打开。

- [ ] **步骤 5：Commit**

```bash
git add src/assets/js/demo.js
git commit -m "feat: 集成一键演示并收尾演示概览"
```

---

### 任务 6：文档与两份演示的同步说明

**文件：**
- 修改：`README.md`、`demo/README.md`、`src/assets/js/demo.js`（文件头注释）
- 不修改：`demo/websocket2serial-demo.html`（本轮保持原样）

- [ ] **步骤 1：更新根 README**

把「全功能演示页」条目改为：

```markdown
- 管理页内置「功能演示」tab（浅色统一主题，7 个子 tab：演示概览/端口/会话/双端口终端/调试/边界自检/事件日志）。
- 独立零构建演示页 `demo/websocket2serial-demo.html` 仍保留（深色、自带连接控件，用法见 `demo/README.md`）。
- 两份演示功能需同步维护；集成版不提供管理页开关，一键演示会跳过「关闭管理页」步骤。
```

- [ ] **步骤 2：更新 `demo/README.md`**

增加「与管理页集成版的关系」一节：管理页打开 `http://127.0.0.1:8766/` → 「功能演示」；两份共用协议实现思路，但 UI 主题、连接方式、管理页开关步骤不同；修改功能时两处都要改，并各自跑冒烟。

- [ ] **步骤 3：加交叉引用注释**

`src/assets/js/demo.js` 文件头：

```js
// 管理页集成版；独立深色版见 demo/websocket2serial-demo.html，功能变更需同步。
```

- [ ] **步骤 4：Commit**

```bash
git add README.md demo/README.md src/assets/js/demo.js
git commit -m "docs: 说明管理页集成演示与独立演示页的关系"
```

---

### 任务 7：最终验证

- [ ] **步骤 1：运行格式、Lint、全量测试**

```powershell
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

预期：全部通过，无 warning。

- [ ] **步骤 2：运行 JS 模块语法检查**

```powershell
$tmp = Join-Path $env:TEMP 'ws2s-check.mjs'
Get-ChildItem src\assets\js\*.js | ForEach-Object { Copy-Item $_.FullName $tmp -Force; node --check $tmp; if ($LASTEXITCODE -ne 0) { throw $_.Name } }
```

- [ ] **步骤 3：浏览器冒烟（按规格第 10.2 节）**

1. `cargo run`，打开 `http://127.0.0.1:8766/`，默认「概览」。
2. 三个主 tab、七个演示子 tab 均可切换，外观为统一浅色。
3. 打开两个互连串口，双向发送与 4KB 回环；切走再切回，终端日志与数据 WS 仍在。
4. 串口助手与演示调试同时订阅，replay 与帧流互不覆盖。
5. 一键演示 `admin` 步骤为 skip；失败路径清理无残留。
6. 1050px 以下双端口终端折叠，无横向溢出。
7. Chrome 109 / Firefox 115 ESR 控制台无未捕获错误。

- [ ] **步骤 4：修复发现的问题并补提交（仅当有修复时）**

```bash
git add -A
git commit -m "fix: 最终验证修复"
```
