# 原生管理窗口实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法跟踪进度。

**目标：** Windows 双击 exe 后显示内嵌现有管理页的原生窗口，支持最小化/关闭到托盘、托盘恢复、单实例唤醒及静默自启。

**架构：** 保留 Tokio 服务主循环与 Win32 托盘，新增独立 Windows UI 线程承载 Win32 窗口和 WebView2。主循环通过命令通道控制窗口；纯窗口状态和导航策略独立测试。Linux 保持现状。

**技术栈：** Rust 1.77.2、Win32/windows-sys、兼容 Win7 的 WebView2 SDK/Runtime、现有 Axum 管理页。

**规格：** 本计划实现用户在 2026-10-28 批准的「原生管理窗口」方案；原设计 `2026-10-25-websocket2serial-design.md` 中“不提供原生桌面窗口”约束由本次批准取代。

## 全局约束

- Windows 7 SP1 x64 与 Rust 1.77.2 支持不回归。
- 管理页继续使用 `127.0.0.1`，不重写现有 HTML/JS 管理功能。
- 窗口最小化与关闭只隐藏；托盘“退出”才终止进程。
- WebView2 缺失不得影响核心 WS、串口和托盘功能。
- Win7 仅使用 WebView2 Runtime 109 与 SDK 1.0.1518.46 或兼容旧版本。
- 自启默认隐藏窗口，手动启动显示窗口。
- 不提交 WebView2 Runtime 二进制。

## 审查重点（Review Focus）

- 关闭/最小化窗口不得销毁 WebView2 主窗口或停止串口会话。
- WebView2 创建失败、运行时缺失、导航失败必须可降级且可再次从托盘打开。
- 管理服务运行时关闭/重开不得让窗口停留在失效页面或失去恢复入口。
- 第二实例、托盘重复打开与退出竞争不得创建多个服务或导致 UI 线程悬挂。
- 外部导航不得在内嵌窗口中突破本机管理页边界。

---

### 任务 1：窗口状态与导航策略

**文件：**
- 创建：`src/desktop/state.rs`
- 创建：`src/desktop/mod.rs`
- 创建：`tests/desktop_state.rs`
- 修改：`src/lib.rs`

- [ ] 先写状态测试：初始隐藏、显示、最小化、关闭、恢复、退出，以及管理页 URL 的允许/拒绝规则。
- [ ] 运行 `cargo test --test desktop_state`，确认因模块/行为缺失而失败。
- [ ] 实现最小纯状态与 URL 策略。
- [ ] 重跑测试并运行全套 `cargo test`。
- [ ] 提交 `feat: add desktop window state model`。

### 任务 2：Windows 原生窗口与 WebView2

**文件：**
- 创建：`src/desktop/windows.rs`
- 修改：`src/desktop/mod.rs`
- 修改：`src/paths.rs`
- 修改：`Cargo.toml`、`Cargo.lock`

- [ ] 锁定通过 Rust 1.77.2/Win7 兼容性验证的 WebView2 绑定，记录版本决策。
- [ ] 实现 UI 线程、窗口创建、WebView2 初始化、resize、show/hide、shutdown。
- [ ] 限制内嵌导航；新窗口及外部地址交系统浏览器。
- [ ] WebView2 用户数据写入 `%LOCALAPPDATA%`，创建失败返回错误而不退出核心服务。
- [ ] 运行 `cargo +1.77.2 check` 与 `cargo test`。
- [ ] 提交 `feat: host admin page in native window`。

### 任务 3：主程序与托盘联动

**文件：**
- 修改：`src/main.rs`
- 修改：`src/tray/mod.rs`、`src/tray/windows.rs`
- 修改：`src/desktop/mod.rs`

- [ ] 启动后按配置显示窗口；托盘打开时先启用管理服务再恢复窗口。
- [ ] 增加“在浏览器中打开”，保留原浏览器入口。
- [ ] 管理服务关闭时隐藏窗口，重开可恢复。
- [ ] 退出按 UI → 托盘 → WS/会话 → HTTP 顺序清理，无 UI 线程死等。
- [ ] 运行 `cargo test` 和 Windows 人工冒烟。
- [ ] 提交 `feat: integrate native window with tray`。

### 任务 4：单实例、自启与控制台

**文件：**
- 修改：`src/cli.rs`
- 修改：`src/main.rs`
- 修改：`src/platform/autostart.rs`
- 修改：`src/platform/single_instance.rs`
- 修改：`tests/autostart.rs`、`tests/single_instance.rs`

- [ ] 先增加 `--minimized` 与自启参数测试并确认失败。
- [ ] 第二实例唤醒已有窗口，失败时回退原管理页逻辑。
- [ ] Windows 发布版不显示控制台，同时保持 CLI 输出和错误提示可用。
- [ ] 自启携带 `--minimized`，手动启动正常显示。
- [ ] 运行 `cargo test`。
- [ ] 提交 `feat: refine desktop startup behavior`。

### 任务 5：打包、文档与验收

**文件：**
- 修改：`scripts/package-windows.ps1`
- 修改：`tests/packaging.rs`
- 修改：`README.md`、`docs/deployment.md`
- 修改：`docs/superpowers/specs/2026-10-25-websocket2serial-design.md`
- 创建：`docs/notes/native-window-acceptance.md`

- [ ] 打包脚本支持固定 WebView2 Runtime 目录、版本与校验，不提交运行时。
- [ ] 文档说明 Runtime 109/Win7、包体积、缺失降级、窗口/托盘/CLI 行为。
- [ ] 运行 `cargo +1.77.2 test`、Windows release 构建与打包检查。
- [ ] 在 Windows 11 与 Win7 SP1 执行人工验收并记录未能实测的项目。
- [ ] 提交 `docs: document native window deployment`。
