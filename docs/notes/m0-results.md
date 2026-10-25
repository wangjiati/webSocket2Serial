# M0 可行性验证结果（spike）

- 日期：2026-10-25
- 执行者：自动化子代理（无 Win7 虚拟机 / Ubuntu 18.04 / 物理 USB 转串口）
- 执行机器：Windows 11（本机），Rust 默认 1.97.1，另装 1.77.2
- 结论摘要：**Rust 1.77.2 可构建 Windows x64 静态 CRT 可执行文件，且无需额外 VC 运行库（无 vcruntime140 / api-ms-win-crt 依赖）；托盘 + 气泡、最小 WS 回显在本机 Windows 11 上可启动并跑通。** 但计划中的依赖写法（`tokio = "1.38"` 等 caret 版本）会解析到要求 rustc 1.85 的新版本，必须锁定精确版本。Win7 实机、写超时实测、Chrome 109、Ubuntu 18.04、115200 延迟均无法在本环境执行，标记为 BLOCKED / 待人工。

---

## 0. 结论速览

| 项 | 结果 | 说明 |
|---|---|---|
| Rust 1.77.2 可用 | ✅ 可构建 | 需锁定依赖版本（见 §3） |
| Windows x64 静态 CRT 可运行 | ✅ 本机 Win11 可运行 | 无 vcruntime140/msvcp140/api-ms-win-crt 依赖；Win7 实机未验证 |
| 托盘方案（Win32 `Shell_NotifyIcon`） | ✅ 本机 Win11 托盘 + 气泡正常 | Win7 实机未验证 |
| 写超时方案 | ⚠️ 代码审查确认，实测 BLOCKED | Windows 走 `WriteTotalTimeoutConstant`，Linux 走 `ppoll(POLLOUT)`；无串口硬件无法实测 |
| 127.0.0.1 vs localhost | ✅ 已实测 | `localhost` 先解析 `::1`（IPv6），依赖客户端回退；应统一用 `127.0.0.1` |
| Ubuntu 18.04 构建 | ⏸️ BLOCKED（无环境） | 命令清单见 §6 |
| 115200 内部延迟 | ⏸️ BLOCKED（无硬件） | 人工步骤见 §7 |

---

## 1. 环境

| 项 | 值 |
|---|---|
| 本机 OS | Windows 11（非 Win7） |
| 默认 Rust | 1.97.1 |
| 安装的 1.77.2 | `rustc 1.77.2 (25ef9e3d8 2024-04-09)` / `cargo 1.77.2 (e52e36006 2024-03-26)` |
| 目标 | `x86_64-pc-windows-msvc` |
| 串口 | 无（`Win32_SerialPort` WMI 为空，spike 枚举结果 `(none)`） |
| 容器/虚拟机 | 无 Docker、无 Win7 VM、WSL 仅 Ubuntu-24.04（非 18.04） |
| 浏览器 | 本机无 Chrome 109（未测浏览器场景，用 curl 8.21 / Node 26 原生 WebSocket 代替） |

---

## 2. spike 实现（`spike/`，已 gitignore，不进版本库）

文件：`spike/Cargo.toml`、`spike/src/main.rs`。

四件事，按简报：
1. 打开 `SERIAL_PORT` 指定串口（`SERIAL_BAUD` 默认 115200，8N1 无流控；未设置则跳过）。
2. `127.0.0.1:8765/ws` WebSocket：无串口时二进制回显（echo）；有串口时桥接（客户端帧→串口，串口字节→广播给所有 WS 客户端）；`SPIKE_ECHO=1` 可强制回显。
3. Win32 托盘图标 + 气泡（`Shell_NotifyIconW`，专用线程跑消息循环，仅 Windows）。
4. 收发日志打到 stdout（`[spike] ws rx / serial tx / serial rx ...`）。

写超时：每次写之前调用 `serialport` 的 `set_timeout(SERIAL_WRITE_TIMEOUT_MS 默认 5000ms)`，并打印写耗时，用于观察写停滞行为。读线程每次读之前把超时切成 5ms（spike 简化：读写共用同一句柄，超时按操作切换）。

依赖（`spike/Cargo.toml` 关键项）：
```toml
tokio = { version = "=1.38.0", features = ["rt-multi-thread","macros","signal","net"] }
axum = { version = "0.7", features = ["ws"] }
serialport = "4.5"
hyper-util = "=0.1.20"   # 0.1.21 起用 edition2024，要求 Cargo>=1.85
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.52", features = ["Win32_UI_WindowsAndMessaging","Win32_UI_Shell","Win32_Foundation","Win32_Graphics_Gdi","Win32_System_LibraryLoader"] }
```

---

## 3. Rust 1.77.2 Windows x64 静态 CRT 构建

### 3.1 命令（计划原文 + 依赖锁定）

```powershell
rustup toolchain install 1.77.2
$env:RUSTFLAGS="-C target-feature=+crt-static"
cargo +1.77.2 build --release --manifest-path spike/Cargo.toml --target x86_64-pc-windows-msvc
```

### 3.2 遇到的依赖 MSRV 问题（关键发现）

**问题 1 —— `hyper-util` 用 edition2024。** 首次构建报错：

```
error: failed to parse manifest at ...\hyper-util-0.1.21\Cargo.toml
  feature `edition2024` is required
  ... not stabilized in this version of Cargo (1.77.2 ...)
```

原因：`axum 0.7` 依赖 `hyper-util = "0.1"`（caret），解析到 0.1.21，该版本改用 edition 2024（需 Cargo ≥ 1.85）。

**问题 2 —— `tokio` caret 解析到 1.54。** 锁定 hyper-util 后再构建报错：

```
error: package `tokio v1.54.1` cannot be built because it requires rustc 1.85 or newer
```

原因：计划写 `tokio = "1.38"` 是 `^1.38`，会解析到最新 1.54.1（MSRV 1.85）。

### 3.3 解决方案与最终结果

锁定精确版本：`tokio = "=1.38.0"`、`hyper-util = "=0.1.20"`。其余依赖（mio 1.2.4 / socket2 / bytes / serde / http / hyper 1.12 / axum 0.7.9 / serialport 4.10.1 等）经逐一核对 crates.io `rust_version`，均 ≤ 1.77。

最终构建成功：

```
   Compiling spike v0.1.0 (...)
    Finished release [optimized] target(s) in 20.48s
```

产物：`spike\target\x86_64-pc-windows-msvc\release\spike.exe`，大小 **1,619,968 字节**。

### 3.4 静态 CRT 验证（PE 导入表）

解析 PE 导入表，静态导入的 DLL 仅 9 个：

```
ADVAPI32.dll  bcrypt.dll  cfgmgr32.dll  kernel32.dll  ntdll.dll
setupapi.dll  shell32.dll  user32.dll  ws2_32.dll
```

**没有** `vcruntime140.dll`、`msvcp140.dll`、`api-ms-win-crt-*.dll`（VC 运行库/UCRT 导入为 NONE）。说明 `-C target-feature=+crt-static` 生效，CRT/UCRT 已静态链接；剩余 9 个均为 Win7 SP1 x64 自带系统 DLL（`bcrypt` 由 Rust std 用于随机数，`cfgmgr32`/`setupapi` 来自 serialport 枚举）。注：二进制内还含 `dbghelp.dll` 字符串，但它是 Rust std 崩溃回溯时按需 `LoadLibrary` 加载，非静态导入。**预期可满足“不依赖额外 VC 运行库即可启动”，但必须在 Win7 SP1 虚拟机实测确认（见 §8）。**

### 3.5 本机（Win11）最小启动检查

运行 `spike.exe`（未设 `SERIAL_PORT`，echo 模式）输出：

```
[spike] available serial ports:
  (none)
[spike] SERIAL_PORT not set; serial disabled (echo mode)
[spike] WS listening on ws://127.0.0.1:8765/ws (bind 127.0.0.1 only)
[spike][tray] tray icon added + balloon shown
```

托盘图标 + 气泡在本机正常出现。**注意：这是 Windows 11 结果，不替代 Win7 验证。**

---

## 4. 写超时行为（代码审查结论；实测 BLOCKED）

本机无 COM 口、无“不读数据的设备”、无硬件流控阻塞场景，**无法实测**。以下结论来自对 `serialport 4.10.1` 源码的审查（`src/windows/com.rs`、`src/posix/tty.rs`、`src/posix/poll.rs`）。

### 4.1 Windows

`set_timeout(Duration)` → `SetCommTimeouts`，其中：

```rust
WriteTotalTimeoutMultiplier: 0,
WriteTotalTimeoutConstant: timeout_constant,   // = timeout.as_millis()
```

`write()` 用同步 `WriteFile`，受 `COMMTIMEOUTS` 约束。写停滞（设备不读 / 流控阻塞）时 `WriteFile` 会在 `WriteTotalTimeoutConstant` 毫秒后返回失败，`GetLastError()` 为 `ERROR_SEM_TIMEOUT`，映射为 `io::ErrorKind::TimedOut`。

**结论：Windows 上 `serialport` 的写停滞会在设定的超时后返回 `TimedOut`（由 OS 保证），写超时可用。**

### 4.2 Linux（Ubuntu 18.04 预期）

`set_timeout` 只把超时存入 `self.timeout`；`write()` 先 `ppoll(POLLOUT, timeout)` 等待输出缓冲可写，超时返回 `TimedOut`，然后才 `write(2)`。因此“输出缓冲被流控/不读设备占满”的完全停滞场景会在超时后返回；但 `poll` 一旦可写，随后的 `write(2)` 可能是**部分写**（返回写入字节数），需要上层处理部分写。

**结论：Linux 上写停滞通过 `ppoll` 超时兜底，同样可用，但要处理部分写。**

### 4.3 关键实现约束（对后续 M1 的影响）

`serialport` 的 `set_timeout` 是**读写共用的单一超时**（Windows 一个 `timeout_constant` 同时给读和写；Linux 一个 `self.timeout` 同时给 `wait_read_fd`/`wait_write_fd`）。要同时满足“读 5ms / 写 5s”，必须像 spike 那样**在每次读/写之前分别 `set_timeout`**，或使用独立句柄。生产实现（计划 §7.5/§11.3）已经按“分块写 + 每块写超时”设计，方向正确。

### 4.4 待人工实测步骤

1. Win7 SP1 虚拟机装 com0com，建 COM 对；一端用 pyserial 打开但**不读**且开启硬件流控（或直接不读 + 触发流控阻塞）。
2. 另一端用 spike 打开（`SERIAL_PORT=COMx`、`SERIAL_WRITE_TIMEOUT_MS=2000`），通过 WS 客户端持续发送大块二进制。
3. 观察日志 `[spike] serial write returned error after ~2.0s (kind=TimedOut)`，确认写停滞在超时后返回而非永久挂住。
4. Linux（18.04）用 `socat` PTY 对重复同样实验（PTY 对侧不读）。

---

## 5. `127.0.0.1` vs `localhost`（已实测）

### 5.1 实测命令与结果

spike 只绑定 `127.0.0.1:8765`（IPv4）。用 Node 26 原生 WebSocket 连接：

```
[127.0.0.1] ws://127.0.0.1:8765/ws -> CONNECTED in 27ms, echo OK, bytes=[de ad be ef 0 1]
[localhost]  ws://localhost:8765/ws  -> CONNECTED in 21ms, echo OK, bytes=[de ad be ef 0 1]
```

用 curl 8.21（`curl -v ws://localhost:8765/ws`）观察解析顺序：

```
* Host localhost:8765 was resolved.
* IPv6: ::1
* IPv4: 127.0.0.1
*   Trying [::1]:8765...      ← 先试 IPv6
*   Trying 127.0.0.1:8765...  ← 回退到 IPv4
* Established connection to localhost (127.0.0.1 port 8765)
```

Python `getaddrinfo('localhost')` 返回顺序：`::1`（AF_INET6）在前，`127.0.0.1`（AF_INET）在后。本机向 `::1` 无监听端口的连接失败耗时约 **2 秒**（非立即拒绝）。

### 5.2 结论

- `localhost` 优先解析到 IPv6 `::1`。服务端只绑 IPv4 `127.0.0.1` 时，`localhost` 能否连通**取决于客户端是否回退**（Node/curl 都会回退，本机可通）。
- 本机 `::1` 连接失败的代价约 2s（可能因客户端/系统而异），短超时客户端可能直接失败。
- 这正是规格“统一用 `127.0.0.1`，不用 `localhost`”的原因。**生产网页/管理页/数据 URL 继续硬编码 `127.0.0.1`。**

### 5.3 待人工：Chrome 109（Win7）

本机无 Chrome 109。人工步骤：
1. Win7 SP1 虚拟机安装 Chrome 109 与 Firefox 115 ESR。
2. 打开 `ws://127.0.0.1:8765/ws` 与 `ws://localhost:8765/ws`（可用浏览器控制台 `new WebSocket(...)`），分别记录连接成功/失败与耗时。
3. 重点记录 `localhost` 在 Chrome 109 下是否先尝试 `::1` 导致失败或明显变慢。

---

## 6. Ubuntu 18.04 构建（BLOCKED，命令清单）

本机无 Docker/WSL-18.04，无法执行。以下是照做即可的命令（与计划步骤 5 一致）：

```bash
# 1. 获取 Ubuntu 18.04 环境（任选其一）
#    docker run --rm -it -v "$PWD":/src ubuntu:18.04 bash
#    或 18.04 虚拟机 / 物理机

# 2. 安装构建依赖（serialport 在 Linux 需要 libudev）
apt-get update
apt-get install -y build-essential pkg-config libudev-dev socat curl

# 3. 安装 Rust 1.77.2
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.77.2
. "$HOME/.cargo/env"

# 4. 构建（同一 spike，注意 spike 的 Cargo.toml 已有 hyper-util/tokio 锁定）
cd /src
cargo +1.77.2 build --release --manifest-path spike/Cargo.toml

# 5. 用 socat 建 PTY 对验证串口 + WS 回显
socat -d -d pty,raw,echo=0 pty,raw,echo=0 &
# 输出形如 /dev/pts/N（记下两个）
SERIAL_PORT=/dev/pts/0 SERIAL_BAUD=115200 ./spike/target/release/spike &
# 用 websocat 或 python websockets 连 ws://127.0.0.1:8765/ws，
# 向 /dev/pts/1 写数据观察 spike 日志的 serial rx -> ws tx
```

注意：Ubuntu 18.04 是 glibc 2.27；必须在 18.04 环境内构建才能保证兼容。WSL 的 Ubuntu-24.04（glibc 2.39）**不能**替代。

---

## 7. 115200 内部延迟（BLOCKED，人工步骤）

无 USB 转串口硬件，无法执行。人工步骤：
1. USB 转串口 TX-RX 短接（或两只互连），记录芯片型号、驱动、latency timer。
2. spike 打开 `SERIAL_PORT`（桥接模式），WS 客户端连 `127.0.0.1:8765/ws`。
3. 连续发送带时间戳的字节流，跑 5 分钟；在 spike 内记录「串口 read 返回 → WS send」与「WS 收到 → 串口 write 返回」的时间差。
4. 统计 p50/p99。目标：工具内部 p99 ≤ 5ms；端到端 115200 回环 p99 ≤ 25ms（观察指标）。921600 视硬件选做。

---

## 8. 人工验证清单（未完成，需 Win7 VM / 硬件）

- [ ] Win7 SP1 x64 虚拟机：复制 `spike.exe`，确认无 VC 运行库也能启动、托盘 + 气泡显示。
- [ ] Win7：com0com 虚拟串口对跑通 WS↔串口桥。
- [ ] Win7：Chrome 109 / Firefox 115 ESR 分别测 `ws://127.0.0.1` 与 `ws://localhost`。
- [ ] Win7 / Linux：写停滞（设备不读或流控阻塞）在设定超时后返回 `TimedOut`（§4.4）。
- [ ] Ubuntu 18.04：按 §6 构建并跑通 PTY + WS 回显。
- [ ] 物理 USB 转串口 TX-RX 回环：115200 延迟 p50/p99（§7）。

---

## 9. 结论与风险

**结论**
1. Rust 1.77.2 能构建本工具所需的 tokio/axum/serialport/windows-sys 技术栈，**但必须锁定依赖版本**（至少 `tokio = "=1.38.0"`、`hyper-util = "=0.1.20"`）；否则 caret 会解析到要求 rustc 1.85 的新版本。任务 1 必须提交 `Cargo.lock` 并固定这些版本。
2. `+crt-static` 生效，exe 无 VC 运行库依赖，预期满足 Win7 免装运行库启动（待 Win7 实测）。
3. Win32 托盘 + 气泡方案可行（Win11 已验证，Win7 待实测）。
4. 写超时：Windows/Linux 均有超时机制，但读写共用单一超时，生产实现需按操作分别设置。
5. `localhost` 会先解析 `::1`，务必统一用 `127.0.0.1`。

**风险**
- 依赖 MSRV 是持续风险：任何未来 `cargo update` 都可能把某个传递依赖抬到要求更高 rustc 的版本，必须靠 `Cargo.lock` + 版本上限锁定（本 spike 已验证此风险真实发生）。
- `-C target-feature=+crt-static` 只解决 VC 运行库；剩余系统 DLL（bcrypt/dbghelp 等）Win7 SP1 自带，但需实测确认无遗漏。
- 写超时在 Windows 依赖驱动实现 `WriteTotalTimeoutConstant`；不同 USB 转串口驱动行为可能有差异，需在目标硬件上实测。
- 本结论中 Win7 / Ubuntu 18.04 / 物理串口 / 延迟均为 BLOCKED，不得视为已验证。

---

## 10. 本任务产物

- 提交：`.gitignore`（已含 `/spike`，无需改动）+ 本文档 `docs/notes/m0-results.md`。
- `spike/` 已 gitignore，不进版本库，验证后删除。
