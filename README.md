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

## 设计文档

- 设计规格：`docs/superpowers/specs/2026-10-25-websocket2serial-design.md`
- 实现计划：`docs/superpowers/plans/2026-10-25-websocket2serial.md`
