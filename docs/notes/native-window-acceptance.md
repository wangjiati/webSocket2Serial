# 原生管理窗口验收记录

日期：2026-10-28
环境：Windows 11 x64；Rust 1.77.2；WebView2 Evergreen。

## 自动化

| 检查 | 结果 |
|---|---|
| `cargo build` | 通过 |
| `cargo test` | 126 passed，0 failed，1 ignored（真实串口） |
| `cargo build --release` | 通过 |
| `cargo test --test packaging` | 通过 |
| `powershell` 解析 `package-windows.ps1` | 0 errors |
| `scripts/package-windows.ps1` 实际执行 | 通过，生成 2,010,324 字节 zip |
| 包内 exe PE subsystem | 2（Windows GUI） |
| 包内 exe `--version`（重定向 stdout） | 输出 `ws2serial 0.1.0` |

## Windows 11 人工冒烟

| 场景 | 结果 |
|---|---|
| 双击/启动显示原生窗口并加载管理页 | 通过 |
| 窗口标题、HTTP 管理页 `/api/config` | 通过 |
| 第二实例启动 | 只唤醒已有窗口，第二进程退出，仍为 1 个实例 |
| 关闭窗口 | 进程存活、窗口隐藏、管理页仍返回 200 |
| 最小化窗口 | 进程存活、窗口隐藏、串口服务继续 |
| 管理服务 `admin.set {enabled:false}` | 窗口隐藏、HTTP 管理页停止 |
| 管理服务关闭后启动第二实例 | 自动重开管理服务并恢复窗口 |
| `--minimized` 启动 | 窗口隐藏、托盘/服务运行、HTTP 200 |
| 托盘 v4 双击（LOWORD=WM_LBUTTONDBLCLK） | 恢复原生窗口 |
| 托盘 v4 右键（LOWORD=WM_RBUTTONUP） | 弹出菜单（枚举到 `#32768`） |
| 内嵌页加载 | 通过（当前 Windows 11 WebView2 Evergreen） |
| 静态 CRT 发布包启动 | 通过，原生窗口出现、HTTP 200 |

## 未覆盖

- Windows 7 SP1 x64 实机：当前环境没有 Win7，未实测。
- 固定 WebView2 Runtime 109 CAB 打包：当前环境没有 CAB，脚本仅完成语法/契约测试。
- 托盘菜单项逐项点击、气泡、自启注册表实际开关：本次验证了 v4 双击与右键菜单弹出，
  未逐项确认每个菜单命令（管理服务、自启、退出、浏览器入口）。
- 真实串口收发回归：`tests/serial_real.rs::real_serial_echo` 按设计需要真实串口，保持 ignored。

Win7 交付前必须补做：Fixed Runtime 109 离线包启动、原生窗口、管理页、串口、托盘、自启。
