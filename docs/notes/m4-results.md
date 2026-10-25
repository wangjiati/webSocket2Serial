# M4 打包与现场验收记录

- 日期：2026-10-25
- 执行者：自动化子代理（任务 16）
- 执行机器：Windows 11（本机），Rust 1.77.2；无 Win7 虚拟机 / Ubuntu 18.04 / Docker /
  物理 USB 转串口 / Chrome 109
- 结论摘要：**打包脚本、安装脚本、浸泡脚本、部署文档与 README 已交付并在本机通过**
  `cargo test` / `fmt` / `clippy` / `py_compile` / `bash -n`；Windows zip 已在本机实际
  构建产出。**现场验收（Win7 SP1、Ubuntu 18.04、真实串口回环、8 小时浸泡、50 次重连、
  921600、自动登录重启、旧浏览器）一律无法由本环境执行，标记为「待人工」，不得视为已验证。**

---

## 1. 本机已执行（自动化）

| 项 | 结果 | 命令 / 输出摘要 |
|---|---|---|
| `cargo test`（全量） | ✅ 全绿 | 各集成测试全部 `ok`；`serial_real`（真实串口）按预期 `1 ignored` |
| `cargo fmt --check` | ✅ | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | ✅ | exit 0，无 warning |
| `cargo test --test packaging` | ✅ 9 passed | 含 install.sh / 打包脚本 / soak.py 的新增检查 |
| `python -m py_compile scripts/soak.py` | ✅ | exit 0 |
| `bash -n packaging/install.sh` | ✅ | exit 0 |
| `bash -n scripts/package-linux.sh` | ✅ | exit 0 |
| `install.sh` 临时 HOME 实测 | ✅ | 二进制复制到 `$HOME/.local/bin/websocket2serial`；`$HOME/.config/autostart/websocket2serial.desktop` 生成；`Exec` 为绝对路径 |
| Windows 打包 | ✅ 本机 | `package-windows.ps1` 成功：`dist/websocket2serial-windows-x86_64.zip`（约 1.87 MB），内含 `websocket2serial/{websocket2serial.exe,README.md,config.toml.example}`；exe 约 4.8 MB，`RUSTFLAGS=-C target-feature=+crt-static` |
| Linux 打包 | ⏸️ 未执行 | 本机无 Docker/Podman，走「打印手动构建步骤」分支；脚本 `bash -n` 通过，容器内构建未跑 |

> Windows 静态 CRT（无 vcruntime140 / api-ms-win-crt 依赖）的 PE 导入表分析在 M0
> （`docs/notes/m0-results.md` §3.4）已用相同 RUSTFLAGS 完成；本任务复用同一命令，
> 未在本机重复解析 PE 导入表。

## 2. 待人工（未执行，不得声称已验证）

| 项 | 状态 | 说明 |
|---|---|---|
| Win7 SP1 x64 实机/虚拟机 | ⏳ 待人工 | 解压 zip、免 VC 运行库启动、托盘 + 气泡、串口、管理页、写超时 |
| Ubuntu 18.04 实机/容器 | ⏳ 待人工 | `package-linux.sh` 容器构建 + `install.sh` 安装 + 登录自启 |
| 自动登录重启 | ⏳ 待人工 | 重启后自启进程 + 网页自动 `session.open` 接回（需工控机自动登录） |
| 管理页旧浏览器 | ⏳ 待人工 | Chrome 109（Win7）与 Firefox 115 ESR 各跑一遍 |
| 串口拔插 | ⏳ 待人工 | 20 次拔插：4001、端口释放、插回通知、重开 |
| 页面刷新 50 次 | ⏳ 待人工 | 每次 `session.open` 接回、旧 token 失效 |
| 断线重连 50 次 | ⏳ 待人工 | 宽限/恢复计数正确、最终端口可释放 |
| 8 小时浸泡 | ⏳ 待人工 | RSS 稳定（波动 <5%）、无会话/线程泄漏 |
| 115200 物理回环计数 | ⏳ 待人工 | TX-RX 短接，`scripts/soak.py` 收发字节一致、无 4005 |
| 写超时实测 | ⏳ 待人工 | 设备不读/流控阻塞，5s 内 4007、端口可重开 |
| 内部延迟 p50/p99 | ⏳ 待人工 | 开启 `logging.level="debug"` 后汇总 `elapsed_us` 日志 |
| 921600 回环 / 调试订阅 | ⏳ 待人工（可选） | 硬件具备时执行，不作 v1 门槛 |
| Ubuntu 托盘 | ⏳ 待人工（可选） | 可选 feature；缺失时用管理页 |

## 3. 现场执行步骤参考

1. **Windows**：解压 `websocket2serial-windows-x86_64.zip` → 双击 exe → 托盘开启自启 →
   管理页验收 → com0com/TX-RX 回环跑 `python scripts/soak.py --port COMx --hours 8 --reconnects 50`。
2. **Linux**：`tar xzf` → `./install.sh --start` → 加入 `dialout` 组 → `socat` PTY 对或
   TX-RX 回环跑 `python scripts/soak.py --port /dev/ttyUSB0 ...`。
3. 延迟：配置 `logging.level="debug"`，跑 5 分钟回环后从日志汇总 `elapsed_us` 的 p50/p99。

失败项：建 issue 并回到对应任务修复（不阻塞本文档）。
