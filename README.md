# WebSocket2Serial

本机常驻的 WebSocket↔串口透传工具：管理 WebSocket 固定 `127.0.0.1:8765`，每个串口
会话动态分配一个独立的二进制数据 WebSocket 端口，并提供一个可开关的本地管理页。

## 平台与工具链

- 目标平台：Windows 7 SP1 x64、Ubuntu 18.04 x86_64。
- MSRV：Rust 1.77.2（`rust-toolchain.toml` 已固定）。
- 依赖版本已按 MSRV 锁定，提交 `Cargo.lock`。

## 构建

```bash
cargo build --release
```

Windows 静态 CRT 与 Ubuntu 18.04 构建细节见
`docs/superpowers/specs/2026-10-25-websocket2serial-design.md` §11.4。

## 打包

- Windows：`powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1`
  （Rust 1.77.2 + 静态 CRT，产出 `dist/websocket2serial-windows-x86_64.zip`）。
- Linux：`bash scripts/package-linux.sh`
  （在 `ubuntu:18.04` 容器内构建，产出 `dist/websocket2serial-linux-x86_64.tar.gz`；
  无 Docker/Podman 时打印手动构建步骤）。

## 安装与启动

### Windows（免安装）

1. 解压 zip 到固定目录（如 `C:\Program Files\WebSocket2Serial\`）。
2. 双击 `websocket2serial.exe`，托盘图标出现。
3. 管理页默认 `http://127.0.0.1:8766/`；串口助手 `/assistant`；功能测试 `/test`；管理 WS `ws://127.0.0.1:8765/ws`。

### Linux

```bash
tar xzf websocket2serial-linux-x86_64.tar.gz
cd websocket2serial
./install.sh            # 复制到 ~/.local/bin 并生成自启项
./install.sh --start    # 安装后立即启动
```

## 管理页与两个工具页

- **管理页** `http://127.0.0.1:8766/`：服务状态、串口列表、会话监控（含计数/断线/错误/关闭）和帮助文档；用新标签打开串口助手与功能测试。
- **串口助手** `http://127.0.0.1:8766/assistant`：自己选择并打开/关闭串口，文本/HEX 收发、时间戳、UTF-8/GBK 接收、快捷指令、发送历史、定时/循环发送、校验和、日志导出；适合日常人工调试。同一串口同时只允许一个数据客户端。
- **功能测试** `http://127.0.0.1:8766/test`：原有 7 个子 tab 的协议演示、边界自检与一键完整演示。
- 管理服务运行时可开可关（托盘菜单 / 网页开关）；关闭后托盘可重开，或重启加 `--admin` 强制开启。
- 命令行：`websocket2serial [--config PATH] [--admin] [--version]`。

## 开机自启

- Windows：托盘菜单开启，写入当前用户 Run 键。
- Linux：`install.sh` 生成 `~/.config/autostart/websocket2serial.desktop`，或托盘菜单切换。
- 自启仅「登录后启动进程」，不打开串口、不恢复会话；串口参数由网页/客户端保存并重新
  `session.open` 接回。重启自动恢复的前提是系统已开启**自动登录**（见 `docs/deployment.md`）。

## 卸载

- Windows：退出进程，删除解压目录与 Run 键
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\WebSocket2Serial`。
- Linux：删除 `~/.local/bin/websocket2serial` 与
  `~/.config/autostart/websocket2serial.desktop`。

## 接入文档与示例

- 协议：`docs/protocol-v1.md`（7 个管理动作、5 个事件、错误码/close code、宽限重连约定）。
- 部署：`docs/deployment.md`（自动登录前提、CSP、`127.0.0.1`、FTDI latency、防火墙、
  崩溃守护边界、日志、升级/回滚、平台依赖）。
- 参考客户端（无构建步骤 ES module）：`clients/ws2serial-client.js`。
- 管理工具：管理页 `/`（状态监控与帮助）、串口助手 `/assistant`（日常调试）、功能测试 `/test`（7 子 tab 演示与一键自检）。
- 独立零构建演示页：`demo/websocket2serial-demo.html`（深色、自带连接控件，用法见 `demo/README.md`）。
- 功能测试页与独立演示页功能需同步维护；功能测试页不提供管理页开关，一键演示会跳过「关闭管理页」步骤。
- 命令行示例：`examples/ws2serial-cli.py`（`--list` / `--port` / `--hex` / `--listen`）。
- 浸泡/回环验收脚本：`scripts/soak.py`（`--hours 8 --reconnects 50`）。

## 设计文档

- 设计规格：`docs/superpowers/specs/2026-10-25-websocket2serial-design.md`
- 管理工具三页面拆分：`docs/superpowers/specs/2026-10-26-admin-tool-split-design.md`
- 实现计划：`docs/superpowers/plans/2026-10-25-websocket2serial.md`
- 三页面拆分计划：`docs/superpowers/plans/2026-10-26-admin-tool-split.md`
- 现场验收记录：`docs/notes/m4-results.md`
