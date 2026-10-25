# WebSocket2Serial 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 subagent-driven-development（推荐）或 executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在 Windows 7 SP1 x64 与 Ubuntu 18.04 上交付一个本机常驻的 WebSocket↔串口透传工具：管理 WS 固定 8765，每个串口会话动态分配二进制数据 WS 端口，带可开关的本地管理页与 Windows 托盘。

**架构：** Rust 单二进制。`SessionManager` 独占管理串口会话的生命周期（awaiting/active/grace/closed），每个会话持有串口读写线程、有界收发队列、调试环形缓冲和独立数据 WS 监听器。管理 WS 用 JSON 提供 7 个动作与 5 个事件；数据 WS 只收发二进制。HTTP 管理页内嵌静态资源，可运行时开关；Windows 托盘用 Win32 API 实现。

**技术栈：** Rust 1.77.2、tokio 1.38、axum 0.7（ws）、serialport 4.5、serde/serde_json、toml 0.8、tracing/tracing-appender、base64、uuid、clap 4.5、windows-sys 0.52（Windows 托盘）、tokio-tungstenite 0.21（测试）。

**规格：** `docs/superpowers/specs/2026-10-25-websocket2serial-design.md`

集成测试统一通过 `mod common;` 引入 `FakeTransport`、`FakeTransportFactory`、`FakeScanner`、`FakeDataListener`、`sample_params()`、`sample_open()`、`sample_port()`、`open_params()`（JSON 形态）、`token_of()`、`wait_closed()` 等助手；测试专用构造器不进入生产 API。

## 全局约束

- Windows 最低 Windows 7 SP1，v1 仅 x64；Ubuntu 最低 18.04 x86_64。
- Windows 使用 Rust 1.77.2 + `-C target-feature=+crt-static`；提交 `Cargo.lock`；不得升级到要求更高 MSRV 的依赖。
- 所有监听硬编码 `127.0.0.1`；配置不提供 `bind`；不做 TLS、不做管理鉴权 token、不做跨网络。
- 串口参数只存在于内存与网页；配置文件只存工具设置，绝不写串口参数。
- 数据 WS 只接受二进制帧；文本帧以 1003 关闭；不保证报文边界。
- 每串口仅一个数据客户端；复用同一端口的非 `active` 会话时轮换 token，旧 URL 立即失效。
- 默认值：连接超时 5000ms、宽限 10000ms、收/发队列各 1 MiB、最大会话 8、读超时 5ms、写停滞超时 5000ms、热插拔轮询 2000ms、心跳 5s/10s。
- 管理协议固定 7 个动作：`ports.list`、`session.open`、`session.close`、`session.list`、`session.write`、`debug.subscribe`、`admin.set`。
- 管理协议固定 5 个事件：`hello`、`ports.changed`、`session.state`、`debug.frame`、`admin.state`。
- 会话计数固定 3 个：`rxBytes`、`txBytes`、`rxDiscardedBytes`，外加 `lastGap`。
- 数据 WS close code 固定：1000、1001、1003、1009、4001、4002、4004、4005、4007。
- 管理页为纯 HTML/CSS/ES2020，无构建步骤、无外部 CDN；兼容 Chrome 109 与 Firefox 115 ESR。
- 管理页开关只改运行时状态，不回写配置；`--admin` 强制本次启动开启。
- 日志按天轮转，保留 7 天；不记录报文内容，不记录 `dataUrl`/token。
- v1 交付 Windows zip 与 Linux tar.gz/install.sh；不做 Windows 安装器与 .deb。
- Linux 托盘为可选 feature，不进入 v1 验收门槛；Windows 托盘为必交。
- v1 不做进程崩溃自动拉起；不做自动重开串口、不缓存/重放串口数据。
- 每个任务结束前必须运行 `cargo fmt --check`、`cargo clippy -- -D warnings`、对应 `cargo test`，并提交。

## 审查重点（Review Focus）

1. **预连接阶段的设备首帧**：`session.open` 后设备立即发 banner，网页稍后才连数据 WS；banner 必须被交付，不能被吞。测试：任务 7 `attach_delivers_pre_attach_banner`，任务 8 数据 WS 回显用例。
2. **网页刷新/旧连接未释放**：同端口在 `awaiting`/`grace` 时必须复用并轮换 token；`active` 时必须 `PORT_BUSY`；旧 URL 必须失效。测试：任务 6 `reuse_rotates_token_without_reopening`、`active_open_is_port_busy`，任务 10 `reuse_rules_through_management_ws`。
3. **串口写停滞**：设备不读或硬件流控阻塞时，写线程不能无限挂住；必须在 5s 内以 4007 销毁会话并释放端口。测试：任务 7 `write_stall_closes_4007`；数据 WS 的 4007 由同一 close watch 路径覆盖。
4. **接收背压**：网页长时间不读导致接收队列满时，必须 4005 明确失败并计数，不能静默丢字节或无限阻塞读串口。测试：任务 7 `rx_backpressure_closes_4005`；数据 WS 的 4005 由同一 close watch 路径覆盖。
5. **管理页锁死与地址**：运行中关闭管理页后必须能通过托盘或 `--admin` 恢复；所有 URL 必须是 `127.0.0.1`，不能用 `localhost`。测试：任务 12 `admin_can_be_disabled_and_reenabled`，任务 1 `origin_allowlist_allows_loopback_and_rejects_remote`，任务 16 手工验收。

## 文件结构

```
Cargo.toml
rust-toolchain.toml
Cargo.lock
.gitignore
README.md
src/
  lib.rs                    公开模块与 AppState
  main.rs                   启动、单实例、CLI、托盘、信号、退出
  cli.rs                    命令行参数
  config.rs                 配置结构、默认值、校验、Origin 白名单
  logging.rs                tracing 初始化、按天轮转
  paths.rs                  配置/日志/锁文件路径
  app.rs                    组装 manager、WS、HTTP、hotplug
  hotplug.rs                串口扫描与 ports.changed
  protocol/
    mod.rs                  Request/Response/Event 顶层类型
    management.rs           7 个动作的参数与返回类型
    events.rs               5 个事件的数据类型
    errors.rs               ErrorCode、ErrorBody、CloseCode
  serial/
    mod.rs                  SerialParams、TransportFactory、PortScanner 导出
    params.rs               串口参数校验
    transport.rs            SerialTransport trait、TransportError
    real.rs                 SystemTransportFactory / RealTransport
    scan.rs                 SystemScanner、PortDiff
  session/
    mod.rs                  导出 Session/Manager 类型
    session.rs              状态、token、计数、调试缓冲、close watch
    manager.rs              独占、open/close/list、复用、定时器
    data_listener.rs        DataListenerFactory trait 与句柄
    io.rs                   读/写线程、分块、背压、错误映射
    counters.rs             Counters / GapInfo
    debug_ring.rs           64 KiB 环形缓冲与 seq
  ws/
    mod.rs                  路由、Origin 校验、带 nodelay 的 accept 循环
    management.rs           管理 WS
    data.rs                 数据 WS
    heartbeat.rs            Ping/Pong 状态机
  http/
    mod.rs                  AdminService、静态资源、/api/config
  tray/
    mod.rs                  TrayAction/TrayStatus/TrayHandle
    state.rs                状态→颜色/提示文本
    windows.rs              Win32 Shell_NotifyIcon
  platform/
    autostart.rs            Windows Run 键 / Linux .desktop
    single_instance.rs      命名互斥量 / 文件锁
  assets/
    index.html
    style.css
    js/api.js
    js/overview.js
    js/assistant.js
    js/main.js
clients/
  ws2serial-client.js       最小参考客户端
examples/
  ws2serial-cli.py          现场/验收命令行示例
tests/
  common/mod.rs             FakeTransport、FakeScanner、FakeDataListener、测试配置
  config.rs
  protocol.rs
  serial_params.rs
  serial_real.rs            #[ignore] 真实串口
  session_state.rs
  session_manager.rs
  session_io.rs
  ws_data.rs
  ws_management.rs
  http_admin.rs
  admin_assets.rs
  tray_state.rs
  single_instance.rs
  packaging.rs              可选：检查安装脚本产物
docs/
  protocol-v1.md
  deployment.md
  notes/m0-results.md
  notes/m4-results.md
scripts/
  package-windows.ps1
  package-linux.sh
  soak.py
packaging/
  install.sh
```

---

## 任务 0：M0 可行性 spike（一次性，不进入产品）

**文件：**
- 创建：`spike/Cargo.toml`、`spike/src/main.rs`、`docs/notes/m0-results.md`
- 修改：`.gitignore`（忽略 `/spike`）

- [ ] **步骤 0：初始化仓库**

```bash
git init
git add docs/superpowers/specs docs/superpowers/plans
git commit -m "docs: 设计文档与实现计划"
```

- [ ] **步骤 1：准备验收环境**

在 Win7 SP1 x64 虚拟机安装 com0com；Ubuntu 18.04 安装 `socat`；制作 USB 转串口 TX-RX 短接回环（或确认可用回显设备）。把 OS、驱动、芯片型号、可用波特率写入 `docs/notes/m0-results.md`。

- [ ] **步骤 2：搭最小 spike**

`spike` 只做四件事：打开 `SERIAL_PORT` 环境变量指定的串口、在 `127.0.0.1:8765/ws` 做 WebSocket 二进制回显、启动 Win32 托盘图标、打印收发日志。使用 `tokio`、`axum`、`serialport`、`windows-sys`；不写测试，不追求结构。

- [ ] **步骤 3：验证 Win7 x64 构建**

运行：

```powershell
rustup toolchain install 1.77.2
$env:RUSTFLAGS="-C target-feature=+crt-static"
cargo +1.77.2 build --release --manifest-path spike/Cargo.toml --target x86_64-pc-windows-msvc
```

把 exe 复制到 Win7 SP1 虚拟机，打开串口、跑通 WS 回显、显示托盘。预期：不依赖额外 VC 运行库即可启动。

- [ ] **步骤 4：验证写超时与地址**

用一只不读数据的设备或硬件流控阻塞场景，记录 `serialport` 写调用是否会在设定超时后返回；记录 Chrome 109 从 `ws://127.0.0.1:8765/ws` 与 `ws://localhost:8765/ws` 连接的成功/失败结果。结果写入 `m0-results.md`。

- [ ] **步骤 5：验证 Ubuntu 18.04**

在 `ubuntu:18.04` 容器安装 `build-essential pkg-config libudev-dev socat`，用 1.77.2 构建并运行同一 spike，确认串口（PTY）与 WS 回显可用。

- [ ] **步骤 6：测 115200 内部延迟**

用 TX-RX 回环，在 spike 内记录“串口 read 返回 → WS send”和“WS 收到 → 串口 write 返回”的时间差；跑 5 分钟，记录 p50/p99。921600 视硬件条件选做。

- [ ] **步骤 7：记录结论**

在 `docs/notes/m0-results.md` 写清：Rust 1.77.2 是否可用、静态 CRT 是否可运行、托盘方案、写超时方案、127.0.0.1/localhost 结论、115200 延迟。若任一项失败，在此任务内决定备选（nightly win7 target / Go 1.20 / 无托盘降级）并停止后续任务。

- [ ] **步骤 8：提交**

```bash
git add .gitignore docs/notes/m0-results.md
git commit -m "chore: 记录 M0 可行性验证结果"
```

`spike/` 不进入版本库；验证完删除。

---

## 任务 1：工程骨架、配置、日志与 CLI

**文件：**
- 创建：`Cargo.toml`、`rust-toolchain.toml`、`.gitignore`、`README.md`、`src/lib.rs`、`src/main.rs`、`src/cli.rs`、`src/config.rs`、`src/logging.rs`、`src/paths.rs`
- 测试：`tests/config.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn defaults_match_spec() {
    let c = Config::default();
    assert_eq!(c.server.management_port, 8765);
    assert_eq!(c.admin.port, 8766);
    assert!(c.admin.enabled);
    assert_eq!(c.session.connect_timeout_ms, 5000);
    assert_eq!(c.session.grace_ms, 10000);
    assert_eq!(c.session.max_sessions, 8);
    assert_eq!(c.session.read_timeout_ms, 5);
    assert_eq!(c.session.write_timeout_ms, 5000);
    assert_eq!(c.session.rx_queue_bytes, 1_048_576);
    assert_eq!(c.session.tx_queue_bytes, 1_048_576);
}

#[test]
fn unknown_config_field_falls_back_to_defaults_with_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[session]\nconnect_timeout_ms = 1\ntyppo = 2\n").unwrap();
    let (cfg, err) = Config::load_or_default(&path);
    assert!(err.is_some());
    assert_eq!(cfg.session.connect_timeout_ms, 5000);
}

#[test]
fn origin_allowlist_allows_loopback_and_rejects_remote() {
    let a = OriginAllowlist::new(&["http://127.0.0.1:*".into(), "http://localhost:*".into()]);
    assert!(a.allows(Some("http://127.0.0.1:3000")));
    assert!(a.allows(Some("http://localhost:8080")));
    assert!(!a.allows(Some("http://evil.example")));
    assert!(!a.allows(None));
}

#[test]
fn cli_admin_flag_is_parsed() {
    let cli = Cli::parse_from(["ws2serial", "--admin"]);
    assert!(cli.force_admin);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test config -v`

预期：FAIL，报 `Config`/`OriginAllowlist`/`Cli` 未定义。

- [ ] **步骤 3：实现骨架**

在 `Cargo.toml` 写入本计划技术栈的固定版本（以 M0 验证过的版本为准，不用 `latest`），`rust-toolchain.toml` 固定 `1.77.2`。运行时依赖加入 `futures-util`、`dirs`；Windows 目标依赖 `winreg`、`windows-sys`；Linux 目标依赖 `fs2`。dev-dependencies 加入 `tempfile`、`futures-util`、`tokio-tungstenite = { version = "0.21", default-features = false, features = ["connect", "handshake"] }`、`reqwest = { version = "0.11", default-features = false, features = ["json"] }`（只测本机 ws/http，不引入 TLS）。`Config` 使用 `serde(deny_unknown_fields)` 与 `Default`；`load_or_default` 在文件缺失时返回默认值，在解析失败时返回默认值 + 错误字符串。`OriginAllowlist` 支持精确匹配与 `prefix:*` 端口通配，拒绝单独 `*` 与 `None`。`paths` 提供 `config_dir()`、`log_dir()`、`lock_path()`。`logging::init` 用 `tracing_appender::rolling::Builder` 按天轮转、`max_log_files(keep_days)`，并同时输出 stderr。`main` 暂时只解析 CLI、加载配置、初始化日志、打印版本后退出。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test config -v`

预期：4 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore README.md src tests/config.rs
git commit -m "feat: 搭建工程骨架、配置、日志与 CLI"
```

---

## 任务 2：管理协议 DTO 与错误码

**文件：**
- 创建：`src/protocol/mod.rs`、`src/protocol/management.rs`、`src/protocol/events.rs`、`src/protocol/errors.rs`
- 测试：`tests/protocol.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn parses_open_request_with_camel_case_params() {
    let raw = r#"{"id":"1","action":"session.open","params":{
        "path":"COM3","baudRate":115200,"dataBits":8,"parity":"none",
        "stopBits":1,"flowControl":"none","dtr":true,"rts":false}}"#;
    let req: Request = serde_json::from_str(raw).unwrap();
    assert_eq!(req.action, ActionName::SessionOpen);
    let p: OpenParams = serde_json::from_value(req.params).unwrap();
    assert_eq!(p.baud_rate, 115200);
    assert_eq!(p.path, "COM3");
}

#[test]
fn serializes_hello_event_with_event_name() {
    let e = Event::Hello(HelloData {
        protocol: "ws2s/v1".into(), version: "0.1.0".into(),
        os: "windows".into(), arch: "x86_64".into(),
        management_port: 8765,
        admin: AdminState { enabled: true, url: "http://127.0.0.1:8766/".into() },
        autostart: true,
    });
    let v = serde_json::to_value(e).unwrap();
    assert_eq!(v["event"], "hello");
    assert_eq!(v["data"]["managementPort"], 8765);
}

#[test]
fn session_selector_accepts_all_or_ids() {
    let all: SessionSelector = serde_json::from_str("\"all\"").unwrap();
    let ids: SessionSelector = serde_json::from_str(r#"["a","b"]"#).unwrap();
    assert!(matches!(all, SessionSelector::All));
    assert!(matches!(ids, SessionSelector::Ids(v) if v.len() == 2));
}

#[test]
fn error_and_close_codes_serialize_as_specified() {
    assert_eq!(serde_json::to_string(&ErrorCode::PortBusy).unwrap(), r#""PORT_BUSY""#);
    assert_eq!(CloseCode::DeviceLost.as_u16(), 4001);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test protocol -v`

预期：FAIL，协议类型未定义。

- [ ] **步骤 3：实现协议类型**

`Request { id: String, action: ActionName, params: Value }`；`ActionName` 为 `rename_all = "snake_case"` 的 7 个变体，另加 `#[serde(other)] Unknown` 让未知 action 走 `UNKNOWN_ACTION` 而不是解析失败。`SessionState` 枚举（`Awaiting/Active/Grace/Closed`，序列化为小写）放在 protocol 层，session 层直接复用，避免两套状态类型。`OpenParams`、`PortInfo`、`SessionInfo`、`Counters`、`GapInfo`、`SessionStateData`、`OpenResult { session_id, data_url, connect_timeout_ms, reused, counters }` 使用 `#[serde(rename_all = "camelCase")]` 与规格字段名。`Event` 用 `#[serde(tag = "event", content = "data", rename_all = "snake_case")]`；5 个事件数据类型与规格一致。`ErrorCode` 用 `SCREAMING_SNAKE_CASE`；`CloseCode` 提供 `as_u16()`。`SessionSelector` 自定义 `Deserialize`，只接受 `"all"` 或字符串数组。`AdminState { enabled, url }`。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test protocol -v`

预期：4 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/protocol tests/protocol.rs
git commit -m "feat: 定义管理协议 DTO 与错误码"
```

---

## 任务 3：串口参数、传输 trait、扫描器与测试替身

**文件：**
- 创建：`src/serial/mod.rs`、`src/serial/params.rs`、`src/serial/transport.rs`、`src/serial/scan.rs`、`tests/common/mod.rs`
- 测试：`tests/serial_params.rs`

- [ ] **步骤 1：编写失败的测试**

`tests/serial_params.rs` 自己定义 `fn sample_params(path: &str) -> SerialParams` 与 `fn sample_port(path: &str) -> PortInfo`，不依赖库里的测试专用方法。

```rust
#[test]
fn rejects_invalid_serial_params() {
    let mut p = sample_params("COM3");
    p.baud_rate = 0;
    assert!(p.validate().is_err());
    p.baud_rate = 115200;
    p.data_bits = 4;
    assert!(p.validate().is_err());
    p.data_bits = 8;
    p.stop_bits = 3;
    assert!(p.validate().is_err());
    p.stop_bits = 1;
    p.path = "".into();
    assert!(p.validate().is_err());
}

#[test]
fn port_diff_detects_added_and_removed() {
    let a = sample_port("COM3");
    let b = sample_port("COM4");
    let d = PortDiff::between(&[a.clone()], &[b.clone()]);
    assert_eq!(d.added, vec![b]);
    assert_eq!(d.removed, vec![a.path]);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test serial_params -v`

预期：FAIL，类型未定义。

- [ ] **步骤 3：实现**

`SerialParams` 与 `OpenParams` 的转换；`validate()` 校验 path 非空、baud>0、data bits 5–8、stop bits 1/2、parity/flow 枚举。`SerialTransport` trait：

```rust
pub trait SerialTransport: Send {
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, TransportError>;
    fn write(&mut self, buf: &[u8], timeout: Duration) -> Result<usize, TransportError>;
    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError>;
    fn set_rts(&mut self, level: bool) -> Result<(), TransportError>;
}
```

`TransportError { Timeout, DeviceLost, WriteTimeout, NotFound, PermissionDenied, Busy, Io(String) }`。`TransportFactory` trait 与 `PortScanner` trait。`SystemScanner` 用 `serialport::available_ports()` 映射 `PortInfo`。`PortDiff::between` 按 `path` 求差集。`tests/common/mod.rs` 提供 `FakeTransport`（可注入读数据、记录写数据、模拟 Timeout/DeviceLost/WriteTimeout/NotFound/PermissionDenied/Busy）与 `FakeTransportFactory`。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test serial_params -v`

预期：2 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/serial tests/serial_params.rs tests/common
git commit -m "feat: 串口参数、传输抽象与扫描器"
```

---

## 任务 4：真实串口实现与写超时

**文件：**
- 创建：`src/serial/real.rs`
- 测试：`tests/serial_real.rs`（真实串口用例 `#[ignore]`）

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn maps_io_errors_to_transport_errors() {
    assert!(matches!(map_io_error(io::ErrorKind::TimedOut), TransportError::Timeout));
    assert!(matches!(map_io_error(io::ErrorKind::BrokenPipe), TransportError::DeviceLost));
    assert!(matches!(map_io_error(io::ErrorKind::NotFound), TransportError::NotFound));
    assert!(matches!(map_io_error(io::ErrorKind::PermissionDenied), TransportError::PermissionDenied));
    assert!(matches!(map_io_error(io::ErrorKind::Other), TransportError::Io(_)));
}

#[tokio::test]
#[ignore = "需要 WS2S_TEST_PORT 指向一只真实/虚拟串口"]
async fn real_serial_echo() {
    let path = std::env::var("WS2S_TEST_PORT").unwrap();
    let mut t = SystemTransportFactory::new(Duration::from_millis(5), Duration::from_secs(5))
        .open(&sample_params(&path)).unwrap();
    t.write(b"ping", Duration::from_secs(1)).unwrap();
    let mut buf = [0u8; 4];
    let n = t.read(&mut buf, Duration::from_secs(1)).unwrap();
    assert_eq!(&buf[..n], b"ping");
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test serial_real -v`

预期：FAIL，`map_io_error`/`SystemTransportFactory` 未定义。

- [ ] **步骤 3：实现**

`RealTransport` 包装 `Box<dyn serialport::SerialPort>`。打开时设置波特率、数据位、校验、停止位、流控、初始 DTR/RTS。`read`/`write` 每次先 `set_timeout(timeout)`；`TimedOut` 映射为 `Timeout`（读）或 `WriteTimeout`（写），设备消失/管道错误映射为 `DeviceLost`，`NotFound`/`PermissionDenied` 直传；Windows 共享冲突/Linux EBUSY 尽力映射为 `Busy`，无法区分时归 `Io`。`SystemTransportFactory::open` 在调用线程阻塞打开（上层用 `spawn_blocking`）。不在这里做分块；分块在会话写线程。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test serial_real -v`

预期：单元测试 PASS，真实串口用例 ignored。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/serial/real.rs tests/serial_real.rs
git commit -m "feat: 真实串口传输与写超时映射"
```

---

## 任务 5：会话状态、计数与调试环形缓冲

**文件：**
- 创建：`src/session/mod.rs`、`src/session/session.rs`、`src/session/counters.rs`、`src/session/debug_ring.rs`
- 测试：`tests/session_state.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn token_rotation_invalidates_old_token() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (closed_tx, _closed_rx) = tokio::sync::mpsc::unbounded_channel();
    let s = Session::new("s1", sample_params("COM3"), SessionConfig::default(), events, closed_tx);
    let old = s.current_token();
    let new = s.rotate_token();
    assert_ne!(old, new);
    assert!(!s.check_token(&old));
    assert!(s.check_token(&new));
}

#[test]
fn counters_keep_invariant() {
    let mut c = Counters::default();
    c.add_rx(100);
    c.add_discarded(30);
    c.add_tx(10);
    assert_eq!(c.delivered(), 70);
    assert_eq!(c.rx_bytes, 100);
    assert_eq!(c.rx_discarded_bytes, 30);
    assert!(c.rx_bytes >= c.rx_discarded_bytes);
}

#[test]
fn debug_ring_is_bounded_and_seq_increases() {
    let mut r = DebugRing::new(64);
    for i in 0..10 {
        r.push(DebugFrame::new("rx", i, i as u8, vec![0u8; 16]));
    }
    let frames = r.snapshot();
    assert!(frames.len() < 10);
    assert!(frames.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test session_state -v`

预期：FAIL，类型未定义。

- [ ] **步骤 3：实现**

`SessionState { Awaiting, Active, Grace, Closed }`；`CloseReason` 与 `CloseCode` 映射（`ClientClosed→1000`、`AdminClosed→4004`、`DeviceLost→4001`、`WriteTimeout/SERIAL_IO→4007`、`Backpressure→4005`、`GraceTimeout→4005?` 使用管理事件，数据 WS 已断开）。`Session::new(id, params, cfg, events: broadcast::Sender<Event>, closed_tx)` 持有 id、params、创建时间、状态、token、Counters、DebugRing、close watch、rx/tx 通道（先只定义，任务 7 接线）与事件发送端。`Session::info()` 生成 `SessionInfo`。`DebugRing` 按字节上限保存 `DebugFrame`，`seq` 从 1 递增；任务 7 的读/写线程在交付/写入时同时 push 环形缓冲并向 `events` 发 `debug.frame`（`replay` 仅回放时置 true）。`Counters::delivered()` 返回 `rx_bytes - rx_discarded_bytes`。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test session_state -v`

预期：3 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session tests/session_state.rs
git commit -m "feat: 会话状态、计数与调试缓冲"
```

---

## 任务 6：会话管理器（open/close/list、复用、定时器）

**文件：**
- 创建：`src/session/manager.rs`、`src/session/data_listener.rs`
- 测试：`tests/session_manager.rs`、修改 `tests/common/mod.rs`

- [ ] **步骤 1：编写失败的测试**

`tests/common/mod.rs` 提供 `test_manager() -> (SessionManager, Arc<FakeTransportFactory>)`、`sample_open(path)`、`token_of(&OpenResult)`。`SessionManager::attach(session_id, token) -> Result<AttachedData, AttachError>`，`AttachedData { rx, tx }` 供数据 WS/测试使用。

```rust
#[tokio::test]
async fn open_returns_awaiting_session_and_binds_data_port() {
    let (mgr, factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(!r.reused);
    assert!(r.data_url.starts_with("ws://127.0.0.1:"));
    assert_eq!(mgr.list().await[0].state, SessionState::Awaiting);
    assert_eq!(factory.open_count(), 1);
}

#[tokio::test]
async fn reuse_rotates_token_without_reopening() {
    let (mgr, factory) = test_manager();
    let first = mgr.open(sample_open("COM3")).await.unwrap();
    let second = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(second.reused);
    assert_eq!(second.session_id, first.session_id);
    assert_ne!(second.data_url, first.data_url);
    assert_eq!(factory.open_count(), 1);
    assert!(mgr.attach(&first.session_id, &token_of(&first)).await.is_err());
}

#[tokio::test]
async fn active_open_is_port_busy() {
    let (mgr, _factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    let err = mgr.open(sample_open("COM3")).await.unwrap_err();
    assert!(matches!(err, ManagerError::PortBusy));
}

#[tokio::test(start_paused = true)]
async fn awaiting_times_out_after_five_seconds() {
    let (mgr, _factory) = test_manager();
    mgr.open(sample_open("COM3")).await.unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(mgr.list().await.is_empty());
}

#[tokio::test]
async fn close_releases_port_and_can_reopen() {
    let (mgr, _factory) = test_manager();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    mgr.close(&r.session_id, CloseReason::ClientClosed).await.unwrap();
    assert!(mgr.list().await.is_empty());
    let again = mgr.open(sample_open("COM3")).await.unwrap();
    assert!(!again.reused);
}

#[tokio::test]
async fn max_sessions_is_enforced() {
    let (mgr, _factory) = test_manager_with_max_sessions(1);
    mgr.open(sample_open("COM3")).await.unwrap();
    let err = mgr.open(sample_open("COM4")).await.unwrap_err();
    assert!(matches!(err, ManagerError::LimitExceeded));
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test session_manager -v`

预期：FAIL，`SessionManager` 未定义。

- [ ] **步骤 3：实现**

`DataListenerFactory` trait：

```rust
pub trait DataListenerFactory: Send + Sync {
    fn bind(&self, session: Arc<Session>) -> Result<DataListenerHandle, DataError>;
}
pub struct DataListenerHandle { pub port: u16, shutdown: Box<dyn FnOnce() + Send> }
```

`SessionManager` 持有 `Arc<dyn TransportFactory>`、`Arc<dyn DataListenerFactory>`、`broadcast::Sender<Event>`、`Mutex<HashMap<path, session_id>>`、`Mutex<HashMap<session_id, Arc<Session>>>`。`open` 先查端口锁：不存在则 `spawn_blocking` 打开串口、创建 session（传入 `events.clone()` 与 `closed_tx`）、绑定数据监听、插入表、发送 `session.state(awaiting)`、启动 5s 定时器；存在且 state 为 `Awaiting/Grace` 则轮换 token、返回 `reused:true`；存在且 `Active` 返回 `PortBusy`。`TransportError` 映射为 `ManagerError`：`NotFound→PortNotFound`、`PermissionDenied→PermissionDenied`、`Busy→PortBusy`、其他→`SerialOpenFailed`。定时器到点后若状态仍为 `Awaiting` 则 `close(SESSION_EXPIRED)`。`close` 幂等，移除会话、释放端口、关闭数据监听、发送 `session.state(closed)`。`attach` 校验 token 并调用 `Session::attach`（任务 7 才启动 I/O），返回 `AttachedData { rx, tx }`。`max_sessions` 超限返回 `LimitExceeded`。Manager 启动时消费所有会话的 `closed_rx`，收到 session id 后统一走 `close` 清理，保证设备掉线/写停滞/背压也能释放端口。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test session_manager -v`

预期：6 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session/manager.rs src/session/data_listener.rs tests/session_manager.rs tests/common/mod.rs
git commit -m "feat: 会话管理器、端口独占与复用"
```

---

## 任务 7：会话 I/O、宽限、写停滞、背压与掉线

**文件：**
- 创建：`src/session/io.rs`
- 修改：`src/session/session.rs`、`src/session/manager.rs`
- 测试：`tests/session_io.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn attach_delivers_pre_attach_banner() {
    let (mgr, fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    fake.push_rx(b"READY\n");
    tokio::time::sleep(Duration::from_millis(50)).await; // awaiting 不读
    let mut attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(1), attached.rx.recv()).await.unwrap().unwrap();
    assert_eq!(bytes, b"READY\n");
}

#[tokio::test]
async fn write_stall_closes_4007_and_releases_port() {
    let (mgr, fake) = test_manager_with_fake_serial();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let mut attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    fake.fail_next_write(TransportError::WriteTimeout);
    attached.tx.send(b"x".to_vec()).await.unwrap();
    let reason = wait_closed(&mgr, &r.session_id).await;
    assert_eq!(reason.code, 4007);
    assert!(mgr.list().await.is_empty());
}

#[tokio::test]
async fn rx_backpressure_closes_4005() {
    let (mgr, fake) = test_manager_with_small_rx_queue();
    let r = mgr.open(sample_open("COM3")).await.unwrap();
    let _attached = mgr.attach(&r.session_id, &token_of(&r)).await.unwrap();
    for _ in 0..10 { fake.push_rx(&[0u8; 4096]); }
    let reason = wait_closed(&mgr, &r.session_id).await;
    assert_eq!(reason.code, 4005);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test session_io -v`

预期：FAIL，I/O 未接线。

- [ ] **步骤 3：实现**

`Session::attach` 首次进入 `active` 时启动两个阻塞线程（共享 `Arc<Mutex<Box<dyn SerialTransport>>>`）：

- 读线程：5ms 超时循环；`active` 时把最多 4096 字节 `try_send` 到容量 `rx_queue_bytes/4096` 的通道，满则 `close(Backpressure, 4005)`；`grace` 时只 `record_rx(discarded)`；`DeviceLost` → 4001，其他 I/O → 4007。
- 写线程：从容量 1 的 tokio mpsc 取 `Vec<u8>`；按约 100ms 线路时间分块（`clamp(baud/100, 256, 4096)`），每块用 `write_timeout_ms` 写；`WriteTimeout` → 4007，`DeviceLost` → 4001；成功 `record_tx`。

`Session` 持有 `Mutex<Option<mpsc::Receiver<Vec<u8>>>>`；数据 WS 在 attach 时 `take`，进入 grace 时归还，重连时再取。读线程在 grace 阶段丢弃，不向通道写入，避免队列被填满。数据断开：close 1000 → `close(ClientClosed)`；其他 → `enter_grace()` 并启动 10s 定时器（任务 6 已有定时器模式，复用）。重连成功 → `resume()`，写入 `lastGap { discardedBytes, durationMs, resumed:true }`。关闭时先置 shutdown、drop tx 发送端、join 线程（带 1s 超时），再释放串口。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test session_io -v`

预期：3 个测试 PASS；再跑 `cargo test --test session_manager` 确认无回归。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/session tests/session_io.rs
git commit -m "feat: 会话读写线程、宽限、写停滞与背压"
```

---

## 任务 8：数据 WebSocket 握手、回显与正常关闭

**文件：**
- 创建：`src/ws/mod.rs`、`src/ws/data.rs`
- 修改：`src/session/data_listener.rs`
- 测试：`tests/ws_data.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn data_ws_echoes_binary() {
    let app = test_app_with_fake_serial().await;
    let open = app.management.open(sample_open("COM3")).await.unwrap();
    let mut ws = connect_data(&open.data_url).await;
    ws.send(Message::Binary(b"ping".to_vec())).await.unwrap();
    app.serial.expect_tx(b"ping").await;
    app.serial.push_rx(b"pong");
    assert_eq!(next_binary(&mut ws).await, b"pong");
}

#[tokio::test]
async fn text_frame_closes_1003() {
    let (mut ws, _) = open_active_data_ws().await;
    ws.send(Message::Text("hello".into())).await.unwrap();
    assert_eq!(close_code(&mut ws).await, 1003);
}

#[tokio::test]
async fn wrong_token_returns_404() {
    let app = test_app_with_fake_serial().await;
    let open = app.management.open(sample_open("COM3")).await.unwrap();
    let bad = open.data_url.replace("/ws/", "/ws/bad");
    assert!(connect_data(&bad).await.is_err());
}

#[tokio::test]
async fn second_client_is_closed_4002() {
    let (mut first, url) = open_active_data_ws().await;
    let mut second = connect_data(&url).await;
    assert_eq!(close_code(&mut second).await, 4002);
    first.send(Message::Binary(b"ok".to_vec())).await.unwrap();
}

#[tokio::test]
async fn oversized_frame_closes_1009() {
    let (mut ws, _) = open_active_data_ws().await;
    ws.send(Message::Binary(vec![0u8; 1024 * 1024 + 1])).await.unwrap();
    assert_eq!(close_code(&mut ws).await, 1009);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test ws_data -v`

预期：FAIL，数据 WS 未实现。

- [ ] **步骤 3：实现**

`AxumDataListenerFactory::bind` 绑定 `127.0.0.1:0`，取端口，启动服务，路由 `GET /ws/:token`，用 `oneshot` 关闭。统一用 `ws::serve_with_nodelay(listener, router, shutdown)`：自定义 accept 循环，每个 `TcpStream` 先 `set_nodelay(true)` 再交给 hyper/axum；管理 WS 也复用这个 helper。握手时校验 `Origin`（任务 1 的 allowlist），token 与 `session.check_token` 匹配；错误 token 返回 404；已 `active` 时升级后以 4002 关闭；`awaiting/grace` 时升级并 `attach`。数据循环先用 `manager.attach(session_id, token)` 取得 `AttachedData { rx, tx }`；读二进制 → `attached.tx.send(data).await`（容量 1，自然背压）；文本 → close 1003；超过 1 MiB → 1009。写任务从 `attached.rx` 取数据，转发二进制；会话关闭 watch 触发时发送对应 close code 并退出；异常断开时通过 `session.return_rx(attached.rx)` 归还接收器，并调用 `manager.enter_grace(session_id)`。正常 close 1000 调用 `manager.close(session_id, ClientClosed)`。`tx_queue_bytes` 当前以“容量 1 × 单帧 1 MiB”落实；若配置改小，握手时拒绝超过该值的帧（1009）。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test ws_data -v`

预期：5 个测试 PASS。另补一个 TX 洪泛用例：客户端持续发小帧、串口端慢速消费，断言写入顺序与字节数完整（允许连接变慢，不允许静默丢 TX）。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ws src/session/data_listener.rs tests/ws_data.rs
git commit -m "feat: 数据 WebSocket 二进制透传"
```

---

## 任务 9：数据 WS 宽限重连、close code 与背压

**文件：**
- 修改：`src/ws/data.rs`、`src/session/session.rs`
- 测试：`tests/ws_data.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn abnormal_drop_enters_grace_and_reattach_reports_gap() {
    let (mut ws, url) = open_active_data_ws().await;
    app.serial.push_rx(b"a");
    assert_eq!(next_binary(&mut ws).await, b"a");
    drop(ws); // 非 1000 断开
    tokio::time::sleep(Duration::from_millis(50)).await;
    app.serial.push_rx(b"b"); // grace 丢弃
    app.serial.push_rx(b"c");
    let mut ws2 = connect_data(&url).await;
    let state = app.management.list().await[0].clone();
    assert_eq!(state.last_gap.unwrap().discarded_bytes, 2);
    app.serial.push_rx(b"d");
    assert_eq!(next_binary(&mut ws2).await, b"d");
}

#[tokio::test]
async fn device_loss_sends_4001() {
    let (mut ws, _) = open_active_data_ws().await;
    app.serial.fail_next_read(TransportError::DeviceLost);
    assert_eq!(close_code(&mut ws).await, 4001);
}

#[tokio::test(start_paused = true)]
async fn grace_timeout_closes_session() {
    let (ws, _) = open_active_data_ws().await;
    drop(ws);
    tokio::task::yield_now().await; // 等服务器进入 grace
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(app.management.list().await.is_empty());
}

#[tokio::test]
async fn admin_close_sends_4004() {
    let (mut ws, _) = open_active_data_ws().await;
    let id = app.management.list().await[0].session_id.clone();
    app.management.close(&id, CloseReason::AdminClosed).await.unwrap();
    assert_eq!(close_code(&mut ws).await, 4004);
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test ws_data -v`

预期：新增用例 FAIL，宽限/close code 未接线。

- [ ] **步骤 3：实现**

异常断开（非 1000）由数据 WS 调用 `manager.enter_grace(session_id)`：manager 更新状态、发 `session.state(grace)`、启动 10s 定时器；读线程继续读并只增加 `rx_discarded_bytes`；接收器归还 session；重连时重新取接收器并 resume，`lastGap` 记录丢弃量与持续时间。设备掉线/写停滞/背压由会话 close watch 触发数据 WS 发送 4001/4007/4005 后关闭；关闭顺序是先让数据 WS 发 close code，再关闭监听器并释放串口。grace 超时销毁会话；管理事件 `session.state(closed)` 带 reason。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test ws_data -v`

预期：全部数据 WS 用例 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ws/data.rs src/session/session.rs tests/ws_data.rs
git commit -m "feat: 数据 WS 宽限重连与故障 close code"
```

---

## 任务 10：管理 WebSocket 核心

**文件：**
- 创建：`src/ws/management.rs`、`src/app.rs`
- 修改：`src/ws/mod.rs`、`src/lib.rs`
- 测试：`tests/ws_management.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn hello_then_ports_list_and_open_list_close() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let hello = next_event(&mut ws).await;
    assert_eq!(hello["event"], "hello");
    let ports = request(&mut ws, "ports.list", json!({})).await;
    assert!(ports["data"]["ports"].is_array());
    let open = request(&mut ws, "session.open", json!({
        "path":"COM3","baudRate":115200,"dataBits":8,
        "parity":"none","stopBits":1,"flowControl":"none"
    })).await;
    assert!(open["data"]["dataUrl"].as_str().unwrap().starts_with("ws://127.0.0.1:"));
    let list = request(&mut ws, "session.list", json!({})).await;
    assert_eq!(list["data"]["sessions"][0]["state"], "awaiting");
    let id = open["data"]["sessionId"].as_str().unwrap();
    let closed = request(&mut ws, "session.close", json!({"sessionId": id})).await;
    assert_eq!(closed["data"]["closed"], true);
}

#[tokio::test]
async fn reuse_rules_through_management_ws() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let a = request(&mut ws, "session.open", open_params()).await;
    let b = request(&mut ws, "session.open", open_params()).await;
    assert_eq!(b["data"]["reused"], true);
    assert_ne!(a["data"]["dataUrl"], b["data"]["dataUrl"]);
    app.attach(a["data"]["sessionId"].as_str().unwrap()).await;
    let c = request(&mut ws, "session.open", open_params()).await;
    assert_eq!(c["error"]["code"], "PORT_BUSY");
}

#[tokio::test]
async fn bad_origin_is_rejected() {
    let app = test_app().await;
    let err = connect_management_with_origin(&app, "http://evil.example").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn invalid_action_returns_error() {
    let mut ws = connect_management(&test_app().await).await;
    let r = request(&mut ws, "nope", json!({})).await;
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "UNKNOWN_ACTION");
}

#[tokio::test]
async fn admin_set_and_session_write_are_dispatched() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    let r = request(&mut ws, "admin.set", json!({"enabled":false})).await;
    assert_eq!(r["data"]["enabled"], false);
    let open = request(&mut ws, "session.open", open_params()).await;
    let id = open["data"]["sessionId"].as_str().unwrap();
    let err = request(&mut ws, "session.write", json!({"sessionId":id,"dataB64":"aGk="})).await;
    assert_eq!(err["error"]["code"], "SESSION_STATE");
    app.attach(id).await;
    let ok = request(&mut ws, "session.write", json!({"sessionId":id,"dataB64":"aGk="})).await;
    assert_eq!(ok["data"]["written"], 2);
    app.serial.expect_tx(b"hi").await;
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test ws_management -v`

预期：FAIL，管理 WS 未实现。

- [ ] **步骤 3：实现**

`AppState { config, manager, admin: Arc<dyn AdminController>, scanner, events, started_at }`；`AdminController` trait 提供 `status()` 与 `set_enabled()`（任务 12 实现，测试用 fake）。管理 WS 用 `ws::serve_with_nodelay` 启动（任务 8 的 helper），`GET /ws` 校验 Origin，升级后发送 `hello`；一个写任务从 `mpsc::channel(256)` 取字符串写 sink；响应用 `try_send`，队列满则 1013 关闭连接；一个读循环解析 `Request`，按 7 个 action 分发；响应带相同 `id`。`ports.list` 调 scanner；`session.open/close/list` 调 manager；`session.write` 校验 active、base64 解码、≤64 KiB、50ms 内送入 tx 队列；`admin.set` 调 `AdminController`；`debug.subscribe` 先记录订阅集（任务 11 接调试帧）。错误统一 `{id,ok:false,error:{code,message,detail?}}`。`app.rs` 提供 `build_app(config, fakes...)` 供测试启动管理 WS 与 HTTP。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test ws_management -v`

预期：5 个测试 PASS。另补一个“管理连接断开不影响数据 WS”的用例：打开并 attach 后关掉管理连接，数据 WS 仍能收发，重连管理后 `session.list` 状态一致。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ws/management.rs src/app.rs src/lib.rs tests/ws_management.rs
git commit -m "feat: 管理 WebSocket 核心动作"
```

---

## 任务 11：状态事件、调试订阅、热插拔与心跳

**文件：**
- 创建：`src/hotplug.rs`、`src/ws/heartbeat.rs`
- 修改：`src/ws/management.rs`、`src/app.rs`
- 测试：`tests/ws_management.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn session_state_events_follow_lifecycle() {
    let mut ws = connect_management(&test_app().await).await;
    let open = request(&mut ws, "session.open", open_params()).await;
    let ev = next_event_of(&mut ws, "session.state").await;
    assert_eq!(ev["data"]["state"], "awaiting");
    app.attach(&open.session_id).await;
    let ev = next_event_of(&mut ws, "session.state").await;
    assert_eq!(ev["data"]["state"], "active");
}

#[tokio::test]
async fn debug_subscribe_replays_then_streams_with_seq() {
    let app = test_app().await;
    app.serial.push_rx(b"before");
    let open = app.management.open(open_params()).await.unwrap();
    app.attach(&open.session_id).await;
    let mut ws = connect_management(&app).await;
    request(&mut ws, "debug.subscribe", json!({
        "sessionIds":[open.session_id],"replay":true
    })).await;
    let replayed = next_event_of(&mut ws, "debug.frame").await;
    assert_eq!(replayed["data"]["replay"], true);
    app.serial.push_rx(b"live");
    let live = next_event_of(&mut ws, "debug.frame").await;
    assert!(live["data"]["seq"].as_u64() > replayed["data"]["seq"].as_u64());
}

#[tokio::test]
async fn ports_changed_is_emitted() {
    let app = test_app().await;
    let mut ws = connect_management(&app).await;
    app.scanner.add(sample_port("COM9"));
    app.hotplug_tick().await;
    let ev = next_event_of(&mut ws, "ports.changed").await;
    assert_eq!(ev["data"]["added"][0]["path"], "COM9");
}

#[test]
fn heartbeat_dead_after_ten_seconds_without_pong() {
    let mut h = Heartbeat::new(Duration::from_secs(5), Duration::from_secs(10));
    h.on_pong(Instant::now());
    assert!(!h.is_dead(Instant::now() + Duration::from_secs(9)));
    assert!(h.is_dead(Instant::now() + Duration::from_secs(11)));
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test ws_management -v`

预期：新增用例 FAIL。

- [ ] **步骤 3：实现**

会话状态迁移向 `broadcast::Sender<Event>` 发 `session.state`；管理 WS 写任务订阅广播，状态/端口/管理事件始终转发，`debug.frame` 只转发给订阅集（`"all"` 覆盖当前及后续）。状态/端口/管理事件 `try_send` 失败 → 以 1013 关闭该管理连接；`debug.frame` 的 `try_send` 失败 → 直接丢弃，靠 `seq` 缺口提示。这样状态事件不会被调试流量挤掉。`debug.subscribe` 先建/更新订阅，再读会话 `DebugRing` 回放（`replay:true`），用 `seq` 去重；调试帧丢失时 seq 跳变。`hotplug` 每 2s 扫描，`PortDiff` 非空时发 `ports.changed`；测试可手动 `tick`。心跳：每 5s 发 Ping，记录最后 Pong 时间；10s 无 Pong 强制关闭连接（数据连接走 4001/grace；测试用可注入的 `Heartbeat` 状态机单测，连接层手工验收）。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test ws_management -v`

预期：全部管理 WS 用例 PASS。另补一个慢消费者用例：订阅调试后客户端不读，灌入大量调试帧，断言管理连接以 1013 关闭、状态事件仍可通过重连 + `session.list` 对齐。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/hotplug.rs src/ws/management.rs src/app.rs tests/ws_management.rs
git commit -m "feat: 状态事件、调试订阅、热插拔与心跳"
```

---

## 任务 12：HTTP 管理服务与开关

**文件：**
- 创建：`src/http/mod.rs`、`src/assets/index.html`、`src/assets/style.css`
- 修改：`src/app.rs`、`src/main.rs`
- 测试：`tests/http_admin.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[tokio::test]
async fn serves_config_and_index() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let cfg: Value = reqwest::get(format!("{}/api/config", svc.url())).await.unwrap().json().await.unwrap();
    assert_eq!(cfg["managementPort"], 8765);
    let html = reqwest::get(svc.url()).await.unwrap().text().await.unwrap();
    assert!(html.contains("WebSocket2Serial"));
    svc.shutdown().await;
}

#[tokio::test]
async fn admin_can_be_disabled_and_reenabled() {
    let svc = AdminService::start(0, 8765).await.unwrap();
    let url = svc.url().to_string();
    svc.set_enabled(false).await.unwrap();
    assert!(reqwest::get(&url).await.is_err());
    svc.set_enabled(true).await.unwrap();
    assert!(reqwest::get(&url).await.unwrap().status().is_success());
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test http_admin -v`

预期：FAIL，`AdminService` 未定义。

- [ ] **步骤 3：实现**

`AdminService { cfg: AdminConfig, management_port: u16, autostart: AtomicBool, state: Mutex<Option<AdminHandle>> }`。`AdminService::new(port, management_port)` 只构造不监听；`async fn set_enabled(bool)` 启停；`start(port, management_port)` 等价于 `new` + `set_enabled(true)`（测试用）。绑定 `127.0.0.1:port`，提供 `/`、`/style.css`、`/js/*`、`/api/config`。它不持有 `AppState`，避免与 `AppState.admin` 形成循环；main 用 `new` 构造，配置启用或 `--admin` 时再 `set_enabled(true)`。静态资源用 `include_str!`。`/api/config` 返回 `{managementPort, version, admin:{enabled,url}, autostart}`；`version` 取 `env!("CARGO_PKG_VERSION")`；`autostart` 由 `set_autostart_status(bool)` 更新，任务 14 接入真实状态，之前默认 `false`。`set_enabled(false)` 触发 shutdown 并清空句柄；`set_enabled(true)` 重新绑定。实现 `AdminController` trait 供管理 WS 的 `admin.set` 调用。启动时按配置决定是否启动；绑定失败只记录状态，不阻止管理 WS。`--admin` 在 main 中覆盖启动默认。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test http_admin -v`

预期：2 个测试 PASS。

- [ ] **步骤 5：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/http src/assets/index.html src/assets/style.css src/app.rs src/main.rs tests/http_admin.rs
git commit -m "feat: HTTP 管理页服务与运行时开关"
```

---

## 任务 13：管理页 UI（概览 + 串口助手）

**文件：**
- 创建：`src/assets/js/api.js`、`src/assets/js/overview.js`、`src/assets/js/assistant.js`、`src/assets/js/main.js`
- 修改：`src/assets/index.html`、`src/assets/style.css`、`src/http/mod.rs`
- 测试：`tests/admin_assets.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn admin_assets_have_required_controls_and_no_cdn() {
    let html = include_str!("../src/assets/index.html");
    for id in ["overview", "assistant", "ports", "sessions", "admin-toggle", "export"] {
        assert!(html.contains(&format!("id=\"{id}\"")), "missing {id}");
    }
    assert!(!html.contains("cdn."));
    assert!(!html.contains("https://unpkg"));
    let api = include_str!("../src/assets/js/api.js");
    assert!(api.contains("127.0.0.1"));
    assert!(api.contains("reconnect"));
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test admin_assets -v`

预期：FAIL，资源文件不存在。

- [ ] **步骤 3：实现**

JS 接口固定为：

- `api.js`：`export async function getConfig()`；`export class ManagementClient { connect(); request(action, params); onEvent(cb); subscribeDebug(sessionIds, replay); close(); }`。自动重连、`hello` 后 `session.list` 对齐、保留订阅并在重连后恢复。
- `overview.js`：`export function bindOverview(root, client, state)`；渲染端口/会话/管理开关/复制地址。
- `assistant.js`：`export function createAssistant(root, client)`；会话选择、`debug.subscribe`（replay）、HEX/ASCII、方向/时间/seq、暂停/清空、`seq` 缺口提示、导出、发送（HEX/文本；默认禁用，勾选“允许写入”并二次确认）。
- `main.js`：读取 `/api/config`，串联 `ManagementClient` 与两个视图。

`index.html`：三块区域与必要 id；`style.css`：无外部资源。所有 URL 固定 `127.0.0.1`。管理页本身不打开数据 WS。客户端调试缓冲上限 5000 帧或 4 MiB，超出丢最旧并提示；导出只导出当前缓冲。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test admin_assets -v`

预期：PASS。

- [ ] **步骤 5：手工验收**

启动桥 + 假串口（或任务 16 的回环），在 Chrome 109/Firefox 115 ESR 检查：概览、端口、会话、开关、助手实时/回放/HEX/ASCII/暂停/清空/导出、调试写保护、断线重连。记录在 `docs/notes/m4-results.md`。

- [ ] **步骤 6：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/assets src/http/mod.rs tests/admin_assets.rs
git commit -m "feat: 管理页概览与串口助手"
```

---

## 任务 14：Windows 托盘、自启与单实例

**文件：**
- 创建：`src/tray/mod.rs`、`src/tray/state.rs`、`src/tray/windows.rs`、`src/platform/autostart.rs`、`src/platform/single_instance.rs`
- 修改：`src/main.rs`、`src/lib.rs`
- 测试：`tests/tray_state.rs`、`tests/single_instance.rs`

- [ ] **步骤 1：编写失败的测试**

```rust
#[test]
fn tray_status_maps_to_expected_color() {
    assert_eq!(TrayStatus::Idle.color(), 0x00808080);
    assert_eq!(TrayStatus::Active.color(), 0x0000A000);
    assert_eq!(TrayStatus::Waiting.color(), 0x00D0A000);
    assert_eq!(TrayStatus::Error.color(), 0x00C00000);
}

#[test]
fn desktop_entry_contains_absolute_exec_and_autostart() {
    let e = desktop_entry(Path::new("/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("Exec=/home/u/.local/bin/websocket2serial"));
    assert!(e.contains("X-GNOME-Autostart-enabled=true"));
}

#[test]
fn single_instance_is_exclusive() {
    let p = std::env::temp_dir().join(format!("ws2s-{}.lock", std::process::id()));
    let a = SingleInstance::acquire_at(&p).unwrap();
    assert!(a.is_some());
    let b = SingleInstance::acquire_at(&p).unwrap();
    assert!(b.is_none());
}
```

- [ ] **步骤 2：运行测试确认失败**

运行：`cargo test --test tray_state --test single_instance -v`

预期：FAIL，类型未定义。

- [ ] **步骤 3：实现**

`TrayStatus` 与颜色/提示文本纯函数放 `state.rs`，先测。`windows.rs` 用 `windows-sys`：专用线程创建隐藏窗口、`Shell_NotifyIconW`、`CreatePopupMenu`/`TrackPopupMenu`、`WM_APP` 回调、`NIF_INFO` 气泡；动作（打开管理页/管理服务开关/自启开关/退出）通过 `tokio::sync::mpsc::UnboundedSender<TrayAction>` 发给 main；状态更新通过 `std::sync::mpsc` 发给托盘线程。图标颜色用 GDI 生成 16×16 HICON（不引入图片资源）。`autostart.rs` 用 `winreg`（Windows）写 `HKCU\...\Run`，Linux 写 `~/.config/autostart/websocket2serial.desktop`；启动时若已启用则刷新当前 exe 路径，并调用 `AdminService::set_autostart_status(...)` 更新 `/api/config`。`windows-sys` 需要 `Win32_UI_Shell`、`Win32_UI_WindowsAndMessaging`、`Win32_Graphics_Gdi`、`Win32_Foundation`、`Win32_System_LibraryLoader` 特性。`single_instance.rs` Windows 用命名互斥量，Linux 用 `fs2` 文件锁；返回 `Option<SingleInstance>`。main 集成：单实例失败则打开管理页并退出；管理 WS 绑定失败视为致命错误（Windows MessageBox / Linux stderr + 日志）；用 `AdminService::new` 构造管理页服务，配置启用或 `--admin` 时 `set_enabled(true)`；启动托盘；退出时停服务、关会话、刷日志。

- [ ] **步骤 4：运行测试确认通过**

运行：`cargo test --test tray_state --test single_instance -v`

预期：3 个测试 PASS。

- [ ] **步骤 5：Win7 手工验收**

Win7 SP1 虚拟机：托盘四色、tooltip、菜单开关、气泡、双击打开管理页、退出释放串口；自启注册表项；移动 exe 后启动会刷新路径。记录到 `m4-results.md`。

- [ ] **步骤 6：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/tray src/platform src/main.rs src/lib.rs tests/tray_state.rs tests/single_instance.rs
git commit -m "feat: Windows 托盘、自启与单实例"
```

---

## 任务 15：参考客户端、CLI 示例与协议文档

**文件：**
- 创建：`clients/ws2serial-client.js`、`examples/ws2serial-cli.py`、`docs/protocol-v1.md`
- 测试：`tests/packaging.rs`（语法/帮助检查，可选依赖）

- [ ] **步骤 1：编写失败的检查**

```rust
#[test]
fn client_and_cli_exist_and_use_loopback() {
    let js = include_str!("../clients/ws2serial-client.js");
    assert!(js.contains("127.0.0.1"));
    assert!(js.contains("session.open"));
    let py = include_str!("../examples/ws2serial-cli.py");
    assert!(py.contains("--port"));
    assert!(py.contains("127.0.0.1"));
}
```

- [ ] **步骤 2：运行检查确认失败**

运行：`cargo test --test packaging -v`

预期：FAIL，文件不存在。

- [ ] **步骤 3：实现**

`ws2serial-client.js`：无构建步骤的 ES module，封装管理 WS + 数据 WS；负责 `session.open`、`client` 侧参数保存、`PORT_BUSY` 退避、数据断开先试原 URL 再 open、管理 WS 重连后 `session.list`、二进制 `binaryType="arraybuffer"`。`ws2serial-cli.py`：`--management-port`、`--port`、`--baud`、`--hex`、`--listen`；列出端口、打开、stdin/stdout 透传、Ctrl-C 关闭。`docs/protocol-v1.md`：逐字段列出 7 动作、5 事件、错误码、close code、复用/token 轮换、重连约定。所有示例固定 `127.0.0.1`。

- [ ] **步骤 4：运行检查确认通过**

运行：`cargo test --test packaging -v`

预期：PASS。若环境有 Node，再运行 `node --check clients/ws2serial-client.js`；若没有，记录跳过。

- [ ] **步骤 5：手工联调**

用 `socat`/com0com 或 TX-RX 回环启动桥，CLI 打开端口、发 HEX、收数据、关闭；参考客户端在管理页同源环境跑通 open/断线重连。记录 `m4-results.md`。

- [ ] **步骤 6：格式化、静态检查、提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add clients examples docs/protocol-v1.md tests/packaging.rs
git commit -m "docs: 协议文档、参考客户端与 CLI 示例"
```

---

## 任务 16：打包、部署文档与现场验收

**文件：**
- 创建：`scripts/package-windows.ps1`、`scripts/package-linux.sh`、`packaging/install.sh`、`scripts/soak.py`、`docs/deployment.md`、`docs/notes/m4-results.md`
- 修改：`README.md`

- [ ] **步骤 1：编写失败的检查**

```rust
#[test]
fn install_script_mentions_desktop_and_binary() {
    let s = include_str!("../packaging/install.sh");
    assert!(s.contains(".config/autostart"));
    assert!(s.contains("websocket2serial"));
}
```

- [ ] **步骤 2：运行检查确认失败**

运行：`cargo test --test packaging -v`

预期：FAIL，脚本不存在。

- [ ] **步骤 3：实现打包与文档**

`package-windows.ps1`：`cargo +1.77.2 build --release --target x86_64-pc-windows-msvc` + 复制 exe/README/config 模板到 zip。`package-linux.sh`：在 `ubuntu:18.04` 容器构建，产出 tar.gz。`install.sh`：复制到 `~/.local/bin`，生成绝对路径的 autostart `.desktop`，可选启动。`soak.py`：通过 CLI/参考协议做 115200 回环计数与重连循环（默认 30 分钟，可 `--hours 8`）。`deployment.md`：自动登录前提、CSP、`127.0.0.1`、FTDI latency、防火墙、崩溃守护边界、日志位置、升级/回滚。`README`：安装、启动、管理页、自启、卸载。

- [ ] **步骤 4：运行检查确认通过**

运行：`cargo test --test packaging -v`、`python -m py_compile scripts/soak.py`、`bash -n packaging/install.sh`

预期：全部通过；在临时 HOME 跑 `install.sh`，确认 `.desktop` 生成且 `Exec` 为绝对路径。

- [ ] **步骤 5：现场验收**

Win7 SP1 x64 与 Ubuntu 18.04：解压/安装、自启、自动登录重启、管理页（Chrome 109/Firefox 115 ESR）、串口拔插、页面刷新 50 次、断线重连 50 次、8 小时浸泡、115200 计数一致；921600 与 Ubuntu 托盘按条件选做。结果写入 `m4-results.md`，失败项建 issue 并回到对应任务修复。

- [ ] **步骤 6：提交**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add scripts packaging docs README.md tests/packaging.rs
git commit -m "chore: 打包、部署文档与现场验收记录"
```

---

## 执行顺序与依赖

- 任务 0 必须先完成；任何一项失败都先解决工具链/托盘/写超时，再进入任务 1。
- 任务 1 → 2 → 3 → 4 是基础设施；任务 5 → 6 → 7 是会话核心；任务 8 → 9 是数据面；任务 10 → 11 是管理面；任务 12 → 13 是管理页；任务 14 是托盘；任务 15 是交付物；任务 16 是打包与验收。
- 任务 7 依赖任务 6 的定时器与 manager；任务 9 依赖任务 8 的数据 WS；任务 11 依赖任务 10 的管理 WS；任务 13 依赖任务 12 的 HTTP 服务。
- 每个任务只依赖前面任务已提交的接口；不要提前实现后续任务的代码。

## 提交约定

- 每个任务一个 commit，消息使用 `feat:` / `fix:` / `chore:` / `docs:` + 中文摘要。
- 不把 `spike/`、临时日志、真实串口测试产物提交进版本库。
- 每个任务提交前必须本地通过 `cargo test`；不允许把失败测试留到下一个任务。
