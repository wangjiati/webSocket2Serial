# WebSocket2Serial 部署说明

本文档面向现场部署与运维，覆盖安装、自启与自动登录、管理页/`--admin`、CSP、
`127.0.0.1`、FTDI 延迟调优、防火墙、进程守护边界、日志、升级与回滚，以及
Windows 7 SP1 / Ubuntu 18.04 的平台依赖。

---

## 1. 交付物

| 平台 | 产物 | 说明 |
|---|---|---|
| Windows 7 SP1 x64 | `websocket2serial-windows-x86_64.zip` | 免安装，解压即可运行 |
| Ubuntu 18.04 x86_64 | `websocket2serial-linux-x86_64.tar.gz` | 解压后运行 `install.sh` |

两个包均含：二进制、`README.md`、`config.toml.example`（Linux 包另含 `install.sh`）。

---

## 2. 安装与启动

### 2.1 Windows（免安装）

1. 解压 zip 到固定目录，例如 `C:\Program Files\WebSocket2Serial\`。
2. 双击 `websocket2serial.exe`（首次监听回环端口时系统可能弹防火墙提示，见 §6）。
3. 管理页默认 `http://127.0.0.1:8766/`；管理 WS `ws://127.0.0.1:8765/ws`。
4. 在托盘菜单开启「开机自启」（写入当前用户 Run 键）。

卸载：退出托盘进程，删除解压目录与注册表 Run 键
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\WebSocket2Serial`。

### 2.2 Linux

```bash
tar xzf websocket2serial-linux-x86_64.tar.gz
cd websocket2serial
./install.sh            # 复制到 ~/.local/bin，生成 ~/.config/autostart/websocket2serial.desktop
./install.sh --start    # 安装后立即启动
./install.sh --no-autostart   # 只安装二进制，不生成自启项
```

`install.sh` 为免 root 用户级安装：二进制进 `~/.local/bin`，自启项进
`~/.config/autostart/`，`Exec` 为二进制绝对路径（含空格等保留字符时按 Desktop
Entry 规范转义）。应用每次启动若检测到自启已启用，会用自身 `current_exe` 刷新
`Exec` 路径，避免安装包移动后失效。

卸载：删除 `~/.local/bin/websocket2serial` 与
`~/.config/autostart/websocket2serial.desktop`。

### 2.3 管理工具页面

- 管理页：`http://127.0.0.1:8766/`（状态监控、串口/会话、帮助文档）。
- 串口助手：`http://127.0.0.1:8766/assistant`（自己打开/关闭串口，日常人工收发调试）。
- 功能测试：`http://127.0.0.1:8766/test`（协议动作、边界自检与一键演示）。
- 管理页用新标签打开另外两个页面。同一串口同时只允许一个数据客户端；遇到 `PORT_BUSY` 时，先在管理页「会话」表关闭占用会话，或关闭其他串口程序。
- 三个页面都不落盘串口数据；导出文件由用户手动触发。

---

## 3. 自动登录前提（重启自动恢复的前提）

工具的自启仅「登录后启动进程」，**不打开串口、不恢复会话**。串口参数由生产网页/
客户端侧保存，登录后网页重连并重新 `session.open` 接回。

因此「重启后网页自动连回」必须同时满足：

1. 工控机/设备已配置**开机自动登录**（Windows 自动登录 / Ubuntu GDM 自动登录）。
2. 工具已开启**开机自启**（托盘菜单，或 Linux `install.sh` 生成 `.desktop`）。
3. 生产网页自身随登录自启，并保存串口参数、断线后按协议约定重连。

> 若现场不允许自动登录（无桌面会话），Linux 可改用 systemd user 服务
> （`Restart=on-failure`）作为常驻手段，但会失去托盘；这是已知取舍，见 §7。

---

## 4. 生产网页接入（CSP 与 127.0.0.1）

- **统一用 `127.0.0.1`，不要用 `localhost`**：`localhost` 可能先解析到 IPv6 `::1`
  导致连接失败或明显变慢。数据 URL 中的端口是动态分配的，网页必须原样使用
  `session.open` 返回的 `dataUrl`。
- 生产网页若设置 CSP，必须允许连回环 WebSocket 且端口通配：
  `connect-src 'self' ws://127.0.0.1:*;`（管理 WS 8765 与动态数据端口都在此范围内）。
- 网页必须由本机 HTTP 提供（`http://127.0.0.1:<port>`），工具默认只放行
  `http://127.0.0.1:*` 与 `http://localhost:*` 两个 Origin；`file://` 页面
  （`Origin: null`）会被拒绝。

完整接入约定（`session.open`、宽限重连、token 轮换、`PORT_BUSY` 退避、close code）
见 `docs/protocol-v1.md` §6；参考实现见 `clients/ws2serial-client.js`。

---

## 5. FTDI / USB 转串口 latency timer 调优

端到端延迟受 USB 转串口芯片与驱动的 **latency timer** 影响。FTDI 等驱动默认可能为
16ms，会显著拉大短报文往返延迟。工具内部延迟（读线程 5ms 轮询）与端到端延迟是两回事，
端到端目标（115200 回环 p99 ≤ 25ms）仅作观察指标。

建议现场在 Windows 设备管理器 → 端口属性 →「延迟计时器 / Latency Timer」中将其调到
最低（FTDI 通常 1ms），Linux 可用 `setserial`/`low_latency` 或设备树配置；具体依芯片
（FTDI / CH340 / CP210x）与驱动而定。921600 高速链路对 latency 更敏感，务必先调低。

---

## 6. 防火墙

- 所有监听硬编码 `127.0.0.1`，**回环连接通常不受防火墙影响**。
- Windows 首次监听时可能弹「允许访问」提示；点「允许」或忽略均可（不开放对外端口）。
- 若现场安全策略强管控，可预放行本工具进程（仅回环），无需开放任何入站端口。

---

## 7. 进程守护边界（已知边界，务必阅读）

- **v1 不做进程崩溃自动拉起**。进程崩溃后网页会持续重试但服务不恢复，需人工重启。
- 若现场要求崩溃自愈，用操作系统 supervisor，**不要自写守护进程**：
  - Windows：任务计划程序（触发器「失败后重启」）。
  - Linux：systemd user 服务（`Restart=on-failure`）。
- 单实例：第二实例不启动服务，只尝试打开已有实例的管理页。强杀后重启可正常重新
  打开串口，网页重连即可恢复。
- 串口写停滞（设备不读/流控阻塞）依赖驱动写超时；若驱动/硬件不遵守超时，极端情况下
  需重启进程释放句柄（见设计规格 §15.4 已知边界）。

---

## 8. 管理页开关与 `--admin`

- 管理页运行时可开可关（托盘菜单 / 网页「管理服务」开关 / 协议 `admin.set`）；
  **运行时开关不回写配置**。
- 管理 WS（8765）常开，是恢复入口；管理页关闭后，托盘可重新打开，或重启时加
  `--admin` 强制本次启动开启。
- 命令行：`websocket2serial [--config PATH] [--admin] [--version]`。

---

## 9. 配置与日志

### 9.1 配置

| 平台 | 配置路径 |
|---|---|
| Windows | `%APPDATA%\WebSocket2Serial\config.toml` |
| Linux | `~/.config/websocket2serial/config.toml`（`$XDG_CONFIG_HOME` 优先） |

首次启动生成默认配置；解析失败时回退默认值并仍能启动（便于进管理页修复）。串口参数
不写入配置。默认值见 `config.toml.example`。

### 9.2 日志

| 平台 | 日志路径 |
|---|---|
| Windows | `%LOCALAPPDATA%\WebSocket2Serial\logs\` |
| Linux | `~/.local/state/websocket2serial/logs\` |

按天轮转、保留 7 天；只记录启动/退出、会话打开/关闭（原因、close code、计数）、
数据连接与宽限、调试订阅/写（仅字节数）等基础事件，**不记录报文内容与 token**。
串口「读返回 → 入队」「写返回」两处耗时以 `tracing::debug` 的 `elapsed_us` 字段输出，
用于验收统计工具内部延迟 p50/p99（需 `logging.level = "debug"` 才可见）。

---

## 10. 升级与回滚

- **升级**：退出旧进程 → 用新包覆盖二进制（Windows 解压覆盖 exe；Linux 重新
  `./install.sh` 覆盖 `~/.local/bin` 下二进制）→ 启动。自启路径会随启动自动刷新。
- **回滚**：用旧版本二进制覆盖回来即可；配置与日志不删除。
- 配置字段向后兼容：未知字段会被忽略并回退默认值，不影响启动。

---

## 11. 平台依赖

### 11.1 Windows 7 SP1 x64

- 构建使用 Rust 1.77.2（最后一个官方支持 Win7 的稳定版），目标
  `x86_64-pc-windows-msvc`，`-C target-feature=+crt-static`：**无需额外安装 VC
  运行库**（无 vcruntime140 / api-ms-win-crt 依赖）。
- v1 仅 x64；32 位 Win7 需现场盘点后另行评估 i686 构建。
- 交付前需在 Win7 SP1 x64 虚拟机实测启动/串口/管理页/托盘/写超时。

### 11.2 Ubuntu 18.04 x86_64

- 在 `ubuntu:18.04` 容器内构建（保证 glibc 2.27 兼容）；构建依赖
  `build-essential`、`pkg-config`、`libudev-dev`，运行时依赖系统自带的 `libudev1`。
- 串口权限：普通用户需加入 `dialout`（或 `uucp`）组，否则打开串口返回
  `PERMISSION_DENIED`。
- Linux 托盘为可选 feature，默认构建不启用；缺失时用管理页，协议与功能不受影响。

---

## 12. 现场验收清单（人工执行）

见 `docs/notes/m4-results.md`。关键项：Win7 SP1 与 Ubuntu 18.04 解压/安装、自启、
自动登录重启、管理页（Chrome 109 / Firefox 115 ESR）、串口拔插、页面刷新 50 次、
断线重连 50 次、8 小时浸泡、115200 计数一致、`elapsed_us` 日志汇总 p50/p99；
921600 与 Ubuntu 托盘按条件选做。
