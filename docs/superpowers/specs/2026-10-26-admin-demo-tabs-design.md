# 管理页集成功能演示（Tab 化）设计

- 日期：2026-10-26
- 状态：已批准（2026-10-26）
- 相关实现：`src/assets/index.html`、`src/assets/style.css`、`src/assets/js/*`、`src/http/mod.rs`

## 1. 背景与意图

管理页目前只有「概览」「串口助手」两个上下堆叠的区域；完整的协议演示仍然是一个独立单文件页（`demo/websocket2serial-demo.html`，深色主题、自带顶栏和连接控件），需要通过 `python -m http.server` 单独打开。现场验收时要在两个页面之间来回切换，看起来不像一个产品。

本次把演示页完整搬进管理页，用两级 tab 组织：管理页顶部是主 tab，演示内部保留原有的 7 个子 tab。视觉上统一成管理页的浅色主题，交互上去掉重复的连接/管理页入口，让管理页成为一个完整、统一的本地工具。

## 2. 目标

1. 管理页顶部提供 `概览 / 串口助手 / 功能演示` 三个主 tab。
2. 「功能演示」内部提供 7 个子 tab：`演示概览 / 端口 / 会话 / 双端口终端 / 调试 / 边界自检 / 事件日志`。
3. 演示页的全部能力（7 个管理动作、5 个事件、数据 WS、双端口终端、调试订阅/回放、边界自检、一键演示、事件导出）在管理页内可用。
4. 演示区使用管理页的浅色色板、按钮、表单、表格、徽标；终端/日志/事件区域保留深色底，与现有「串口助手」日志一致。
5. 管理页与演示共用 `/api/config` 的 `managementPort`，打开页面即自动连接，不再要求用户输入端口或点“连接”。
6. 切换主 tab / 子 tab 不销毁演示状态：已打开的数据 WS、终端日志、调试订阅、事件日志保持。
7. 独立演示页继续保留，作为现场零构建版本。

## 3. 非目标

- 不把演示的 7 个子 tab 摊平到管理页主 tab 栏。
- 不删除或改造 `demo/websocket2serial-demo.html`（保持深色、独立、自带连接控件）。
- 不新增前端构建步骤、框架、CDN 或外部字体。
- 不新增管理协议动作、事件或数据面能力。
- 不做多个浏览器标签页之间的状态同步。
- 不做 URL 深链接（不要求 `#demo/terminal` 之类的地址）。
- 不把「概览」里的端口/会话表与演示子 tab 的端口/会话表合并成一套；概览是摘要，演示子 tab 是可操作的全量视图。

## 4. 反目标

- 集成后仍需手动输入管理端口或点“连接”才能用演示。
- 关闭管理页的按钮仍出现在演示区，导致用户在看演示时把当前页面自身的 HTTP 服务关掉。
- 两份演示代码在没有人知道的情况下各自漂移；README 必须写明同步义务。
- 演示的全局样式（`*`、`body`、`button`、`table`、`.badge` 等）泄漏到概览/串口助手，改变现有页面外观。

## 5. 已确认的决策

| 项 | 决定 | 来源 |
|---|---|---|
| 主 tab 结构 | `概览 / 串口助手 / 功能演示` | 用户 |
| 演示 tab 结构 | 作为子 tab 集成，保持原有 7 个，不摊平 | 用户 |
| 视觉 | 全部浅色页面，看起来是一个完整产品 | 用户 |
| 独立演示页 | 保留两份，同时存在 | 用户 |
| 演示管理连接 | 演示模块单独持有一个管理 WS 实例（与管理页各一条连接） | 建造者 |
| 调试订阅 | 不复用管理页客户端；`debug.subscribe` 按连接隔离，两条连接互不覆盖 | 建造者 |
| 管理页开关 | 演示区移除；概览页保留现有的 `admin.set` 开关与确认 | 建造者 |
| 一键演示的管理页步骤 | 集成版跳过并注明原因；独立版保持完整 | 建造者 |
| 内容区宽度 | 加宽到约 1500px，窄屏沿用现有断点折叠 | 建造者 |
| 日志配色 | 终端/日志/事件区保持深色底，页面其余部分浅色 | 建造者 |

## 6. 信息架构与交互

### 6.1 页面结构

```
header
  h1 + 连接状态徽标
nav.main-tabs
  概览 | 串口助手 | 功能演示
main
  section#overview.tab-panel            （现有内容，仅加 tab 语义）
  section#assistant.tab-panel           （现有内容）
  section#demo.tab-panel
    div.demo-statusbar                  管理 WS / 版本 / 管理页 / 自启
    div#demo-banner                     操作提示（原 #banner）
    nav.demo-tabs
      演示概览 | 端口 | 会话 | 双端口终端 | 调试 | 边界自检 | 事件日志
    section#demo-tab-overview.demo-panel
    section#demo-tab-ports.demo-panel
    section#demo-tab-sessions.demo-panel
    section#demo-tab-terminal.demo-panel
    section#demo-tab-debug.demo-panel
    section#demo-tab-edge.demo-panel
    section#demo-tab-events.demo-panel
```

- 主 tab 与子 tab 均用 `<button role="tab">` + `aria-selected` + `aria-controls`；面板用 `role="tabpanel"`，默认隐藏，激活时显示。支持左右方向键切换。
- 主 tab 切换只改 class 与 `aria-*`；首次进入「功能演示」时调用一次 `demo.onShow()` 触发终端/调试区重绘，之后每次进入都调用，不重建 DOM。
- 子 tab 切换沿用演示页现有逻辑：显示对应 panel，并对 `terminal` / `debug` / `sessions` 做一次渲染。

### 6.2 演示区顶部的去留

移除演示页原有顶栏中的：

- 管理端口输入框与「连接」「重连管理 WS」按钮：改为按 `/api/config.managementPort` 自动连接，状态在 `.demo-statusbar` 展示。
- 「打开管理页」链接：当前页面就是管理页。
- 「切换管理页」按钮：概览页已有带确认的 `admin.set` 开关。
- 演示页 footer 与 `python -m http.server` 的启动说明：集成后不再需要。

保留并改造成浅色状态条：管理 WS 连接状态、工具版本、管理页开关状态、开机自启状态（只读）。状态条位于「功能演示」主面板顶部，不影响另外两个主 tab。

### 6.3 「演示概览」子 tab

- 保留「连接信息」卡片：页面 Origin、管理 WS 地址、协议、工具版本、操作系统、管理页状态、开机自启（只读）、当前会话数。
- 保留「一键完整演示」卡片与步骤列表。
- 移除「打开管理页」「切换管理页」按钮；保留「刷新端口/会话」。
- 底部三张说明卡片保留，但文案改为集成后的实际用法：不再写 `python -m http.server`，不再写 `file://` 警告；继续保留浏览器 close code 限制、虚拟串口 4KB 小块、托盘/自启只读等注意事项。
- 「本页覆盖」文案改为「管理页功能演示覆盖」，避免和主 tab「概览」混淆。

### 6.4 一键完整演示

步骤列表保持不变，但 `admin` 步骤在集成版中的行为改为：

- 状态直接标为 `skip`。
- 详情写「集成在管理页内，跳过关闭自身；可在概览页手动验证」。
- 清理阶段不再调用 `admin.set {enabled:true}`。

其余步骤（连接确认、端口扫描、A/B 选择、打开、数据 WS、双向 4KB、计数、调试、调试写、4002、PORT_BUSY、正常关闭/重开、grace、自启只读、清理）保持现有行为。

### 6.5 数据与连接模型

- `main.js` 创建两个 `ManagementClient` 实例：
  - `adminClient`：给概览与串口助手使用（现有行为不变）。
  - `demoClient`：给演示模块使用。
- 两个实例都连同一个 `config.managementPort`，各自处理 hello、事件、重连和调试订阅。`debug.subscribe` 是连接级状态，分开后「串口助手」与「演示调试」可以各自订阅、各自 replay，互不覆盖。
- 演示模块的端口/会话数据仍来自 `demoClient`；概览/串口助手来自 `adminClient`。两边通过服务端事件各自刷新，允许短暂不同步。
- `api.js` 做最小扩展：`whenReady(timeoutMs)`（hello 到达后 resolve）、`unsubscribeDebug()`；`request()` 继续返回 `data`。演示模块按此适配（例如 `session.write` 结果直接取 `written`）。
- 演示模块使用 `demoClient.onSync` / `onEvent` / `onConnection`，不再自带 `ManagementClient` 类。

## 7. 视觉与样式

### 7.1 主题

- 页面背景、卡片、边框、文字、主色沿用管理页现有 CSS 变量（`--bg`、`--panel`、`--border`、`--text`、`--muted`、`--accent` 等）。
- 演示区的变量、按钮、卡片、徽标、表单、表格样式全部**限定在 `#demo` 作用域下**，例如 `#demo .demo-card`、`#demo .demo-btn`、`#demo .demo-badge`。不新增会影响概览/串口助手的全局选择器。
- 演示原有类名（`.card`、`.btn`、`.badge`、`.kv`、`.tabs` 等）如果保留，统一写成 `#demo .card` 形式；避免与 `style.css` 里已有的 `.badge`、`table`、`button` 规则冲突。
- 终端、日志、事件、边界结果使用深色底（`#0f1115` / `#d1d5db`），与串口助手日志一致；其余区域白底浅色。

### 7.2 布局

- `main` 最大宽度从 1100px 调整为约 1500px；概览与串口助手随容器变宽，表格与日志获得更多空间。
- 「双端口终端」保持两列，沿用 1050px 断点折叠为单列；650px 断点沿用现有移动端规则。
- 主 tab 栏放在 header 下方，横向可换行；子 tab 栏在演示面板内部，样式比主 tab 轻一级。
- 状态条横向排列，窄屏换行。

### 7.3 元素 ID

所有从演示页搬来的元素 ID 统一加 `demo-` 前缀（例如 `ports-body` → `demo-ports-body`，`log-A` → `demo-log-A`，`btn-run-demo` → `demo-btn-run-demo`）。原因：

- 管理页已有 `#ports-body`、`#sessions-body` 等 ID，直接搬会重复。
- 保留 `getElementById`/`querySelector` 的直接映射，减少 JS 改动量。
- `index.html` 中所有 `id` 必须唯一；新增测试会检查这一点。

主 tab 面板继续使用现有 `overview`、`assistant` ID，新增 `demo`。

## 8. 代码结构与文件变更

| 文件 | 变更 |
|---|---|
| `src/assets/index.html` | 增加主 tab 导航；现有两段内容加面板语义；新增「功能演示」面板、状态条、7 个子 tab 及全部演示 HTML（ID 加 `demo-` 前缀）；移除演示原顶栏/footer |
| `src/assets/style.css` | 新增主/子 tab 样式；新增 `#demo` 作用域下的浅色演示样式、状态条、深色日志区；`main` 加宽；移动端断点调整 |
| `src/assets/js/demo.js` | 新建：把原演示脚本改成 ES module，导出 `createDemo(root, client, options)`；去掉自带 ManagementClient、自动连接和顶栏绑定；HTML 选择器改用 `demo-` 前缀；一键演示跳过 admin 步骤 |
| `src/assets/js/main.js` | 创建两个客户端；初始化主/子 tab；绑定概览、串口助手、演示；两边分别 `connect()` |
| `src/assets/js/api.js` | 增加 `whenReady()`、`unsubscribeDebug()`；其余行为不变 |
| `src/http/mod.rs` | 内嵌并分发 `demo.js`（`/js/demo.js`） |
| `tests/admin_assets.rs` | 扩展必需 ID、`demo.js` 分发、HTML ID 唯一性检查 |
| `README.md` | 「全功能演示页」条目改为管理页「功能演示」tab，并注明独立页仍保留 |
| `demo/README.md` | 注明管理页内已有浅色集成版；两份并存，功能变更需同步 |
| `demo/websocket2serial-demo.html` | 不改（保持深色独立版） |

`demo.js` 建议按原脚本的分区组织：工具函数、状态、日志、数据会话控制器、终端、调试、边界、一键演示、UI 绑定、初始化。原脚本约 1600 行，迁移以机械替换为主。

## 9. 错误处理与边界

- 管理 WS 未连接时：状态条显示断开；需要管理 WS 的操作（打开、调试写、一键演示）给出明确错误，不静默失败。
- 演示客户端自动重连沿用 `api.js` 的指数退避；重连后由 `hello` 触发端口/会话对齐，调试订阅由 `subscribeDebug` 记录并在重连后恢复。
- `/api/config` 获取失败：`main.js` 现有逻辑在页头显示「初始化失败」，控制台保留错误；演示区不初始化。
- 浏览器禁用端口：沿用 `BROWSER_BLOCKED_PORTS` 与关闭重开的处理。
- 浏览器只允许 1000/3000-4999 关闭码：保留现有「异常断开用 3001」的说明与实现。
- 一键演示失败：保留现有 fail/skip 标记与清理逻辑；清理不再操作管理页开关。
- 关闭管理页只能在「概览」页操作，且保留确认文案；演示区不再提供该入口。

## 10. 测试与验收

### 10.1 自动化

- `tests/admin_assets.rs`：
  - 必需控制点：主 tab 三个、演示子 tab 七个、`demo-btn-run-demo`、`demo-ports-body`、`demo-sessions-body`、`demo-log-A`、`demo-log-B`、`demo-debug-body`、`demo-event-log`。
  - HTML 中所有 `id="..."` 唯一。
  - 无 `cdn.` / `https://unpkg` / `file://` 依赖。
  - `/js/demo.js` 返回成功、内容非空、`Content-Type` 含 `javascript`。
- JS 语法：对 `api.js`、`main.js`、`overview.js`、`assistant.js`、`demo.js` 做一次模块语法检查（复制为临时 `.mjs` 后 `node --check`）。
- `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test` 全量通过。

### 10.2 手工冒烟

1. `cargo run`，打开 `http://127.0.0.1:8766/`，确认三个主 tab 默认停在「概览」。
2. 切到「功能演示」：状态条自动显示管理 WS 已连接；7 个子 tab 均可切换。
3. 端口页扫描、设为 A/B；会话页打开两个互连串口；双端口终端双向发送；切换主 tab 再回来，终端日志与数据 WS 仍在。
4. 调试页订阅 `all`、产生数据、看到帧与 seq；串口助手同时订阅不被覆盖（各自 replay 正常）。
5. 边界页跑 4002 / PORT_BUSY / 正常关闭等，结果写入边界日志。
6. 一键演示：确认 `admin` 步骤为 skip，其余步骤按现有条件通过或明确提示串口未互连。
7. 窄窗口（<1050px）确认双端口终端折叠、tab 可换行、无横向溢出。
8. 浏览器控制台无未捕获错误；Chrome 109 与 Firefox 115 ESR 各做一次基本切换。

## 11. 风险与取舍

- **两份演示代码漂移**：用户明确要求保留独立页，接受维护两份。README 与两份文件头部互相注明；后续功能变更必须同时改 `demo/websocket2serial-demo.html` 与 `src/assets/js/demo.js`。
- **两条管理 WS 连接**：状态更新可能有毫秒级差异，但避免了调试订阅互相覆盖这一确定性缺陷。管理 WS 承载事件与 JSON 请求，本机双连接成本可忽略。
- **大文件搬迁**：`index.html` 会从约 120 行增长到约 600 行，`style.css` 增加约 400 行，`demo.js` 约 1500 行。保持零构建是现有约束，接受单文件体积。
- **ID 前缀机械替换**：容易漏改。用 HTML ID 唯一性测试 + 浏览器控制台错误 + 手工冒烟兜底。
- **集成版不覆盖 `admin.set` 的一键演示**：概览页仍可手动验证；独立版继续完整覆盖。README 中说明差异。

## 12. 建造者自行决定的部分

- 主/子 tab 的具体间距、字号、激活态样式，以及状态条布局。
- `#demo` 作用域下各类卡片/按钮/徽标的具体浅色取值。
- 演示 DOM 在 `index.html` 中的排列顺序与注释分节方式。
- `createDemo` 的内部拆分粒度；是否把工具函数再拆成独立模块。
- 主 tab 是否记住上次选择（默认不记忆，刷新回到「概览」）。
