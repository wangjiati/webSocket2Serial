#!/usr/bin/env python3
"""WebSocket2Serial 命令行示例（现场/验收用）。

依赖：Python 3.8+，运行「打开/列表」时才需要 `websockets`（惰性 import，
因此 `--help` 在没有安装 `websockets` 时也能正常使用）。

三种用法：
  1) 列表端口：  python ws2serial-cli.py --list
  2) 双向透传：  python ws2serial-cli.py --port COM3 --baud 115200 < in.bin > out.bin
  3) 只收不发：  python ws2serial-cli.py --port COM3 --listen

说明：
  - 固定使用 127.0.0.1（不用 localhost）。
  - 管理动作走管理 WS，数据收发走二进制数据 WS，不自行实现串口。
  - 默认（双向）在 stdin 读到 EOF 后停止发送，并以 1000 正常关闭数据连接，
    串口随之释放；`--listen` 只收不发，运行到 Ctrl-C 或对端关闭。
  - `--hex` 时：stdin 按空白/逗号分隔的十六进制字节解析后写入串口，
    stdout 把收到的字节按大写十六进制打印。
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import json
import sys

DEFAULT_MANAGEMENT_PORT = 8765
DEFAULT_BAUD = 115200

_id_counter = 0


def _next_id() -> str:
    global _id_counter
    _id_counter += 1
    return f"cli-{_id_counter}"


class CliError(Exception):
    """CLI 可预期错误（打印信息后以非零码退出）。"""


def _import_websockets():
    """惰性导入 `websockets`；未安装时抛出带指引的 CliError。"""
    try:
        import websockets
        from websockets.exceptions import ConnectionClosed
    except ImportError as exc:  # pragma: no cover - 依赖环境
        raise CliError(
            "缺少依赖 `websockets`，请先安装：python -m pip install websockets"
        ) from exc
    return websockets, ConnectionClosed


def _connect(url):
    """返回与 `async with` 兼容的 WebSocket 连接（跨 websockets 版本的稳定入口）。"""
    websockets, _ = _import_websockets()
    return websockets.connect(url)


def parse_hex(text: str) -> bytes:
    """把「空白/逗号分隔、可选 0x 前缀」的十六进制文本解析为字节。"""
    cleaned = text.replace(",", " ").replace("0x", " ").replace("0X", " ")
    parts = cleaned.split()
    if not parts:
        return b""
    try:
        return bytes(int(p, 16) for p in parts)
    except ValueError as exc:
        raise ValueError(f"非法 HEX 输入：{text.strip()!r}") from exc


async def _request(ws, action: str, params: dict):
    """在管理 WS 上发请求并等待对应应答（跳过 hello 等事件）。"""
    req_id = _next_id()
    await ws.send(json.dumps({"id": req_id, "action": action, "params": params}))
    while True:
        raw = await ws.recv()
        if isinstance(raw, (bytes, bytearray)):
            raw = bytes(raw).decode("utf-8", "replace")
        msg = json.loads(raw)
        if msg.get("id") == req_id:
            if msg.get("ok"):
                return msg.get("data") or {}
            error = msg.get("error") or {}
            raise CliError(f"{error.get('code', 'UNKNOWN')}: {error.get('message', '未知错误')}")
        # 非本请求的帧（事件）忽略。


async def _list_ports(args) -> None:
    url = f"ws://127.0.0.1:{args.management_port}/ws"
    async with _connect(url) as ws:
        data = await _request(ws, "ports.list", {})
    ports = data.get("ports") or []
    if not ports:
        print("（未发现串口）")
        return
    for port in ports:
        path = port.get("path", "")
        manufacturer = port.get("manufacturer") or ""
        product = port.get("product") or ""
        extra = f"  {manufacturer} {product}".strip()
        print(path if not extra else f"{path}  {extra}")


async def _recv_loop(data, args) -> None:
    _, ConnectionClosed = _import_websockets()
    while True:
        try:
            message = await data.recv()
        except ConnectionClosed:
            return
        if not isinstance(message, (bytes, bytearray)):
            # 数据面应为纯二进制；文本帧忽略（不应出现）。
            continue
        payload = bytes(message)
        if args.hex:
            sys.stdout.write(payload.hex(" ").upper() + "\n")
            sys.stdout.flush()
        else:
            sys.stdout.buffer.write(payload)
            sys.stdout.buffer.flush()


async def _send_loop(data, args, loop) -> None:
    if args.hex:
        reader = sys.stdin
        while True:
            line = await loop.run_in_executor(None, reader.readline)
            if not line:
                return
            try:
                payload = parse_hex(line)
            except ValueError as exc:
                sys.stderr.write(f"{exc}\n")
                continue
            if payload:
                await data.send(payload)
    else:
        reader = sys.stdin.buffer
        while True:
            chunk = await loop.run_in_executor(None, reader.read, 4096)
            if not chunk:
                return
            await data.send(chunk)


async def _passthrough(data, args) -> None:
    loop = asyncio.get_running_loop()
    recv_task = asyncio.create_task(_recv_loop(data, args))
    send_task = None if args.listen else asyncio.create_task(_send_loop(data, args, loop))

    tasks = [recv_task] + ([send_task] if send_task else [])
    try:
        done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    finally:
        for task in pending:
            task.cancel()
        for task in pending:
            with contextlib.suppress(asyncio.CancelledError):
                await task
    for task in done:
        exc = task.exception()
        if exc is not None:
            raise exc


async def _run(args, state: dict) -> None:
    mgmt_url = f"ws://127.0.0.1:{args.management_port}/ws"
    params = {
        "path": args.port,
        "baudRate": args.baud,
        "dataBits": 8,
        "parity": "none",
        "stopBits": 1,
        "flowControl": "none",
    }
    async with _connect(mgmt_url) as mgmt:
        result = await _request(mgmt, "session.open", params)
    state["session_id"] = result.get("sessionId")
    print(
        f"会话 {result.get('sessionId')} 已打开（reused={result.get('reused')}）"
        f"  数据 {result.get('dataUrl')}",
        file=sys.stderr,
    )

    async with _connect(result["dataUrl"]) as data:
        await _passthrough(data, args)
    # 正常退出：数据连接以 1000 关闭，服务端按 ClientClosed 释放串口。


async def _close_session(management_port: int, session_id: str) -> None:
    url = f"ws://127.0.0.1:{management_port}/ws"
    async with _connect(url) as ws:
        await _request(ws, "session.close", {"sessionId": session_id})


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="ws2serial-cli.py",
        description="WebSocket2Serial 命令行示例：通过管理/数据 WebSocket 使用串口。",
        epilog=(
            "示例：\n"
            "  python ws2serial-cli.py --list\n"
            "  python ws2serial-cli.py --port COM3 --baud 115200 --hex\n"
            "  python ws2serial-cli.py --port COM3 --listen"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--management-port",
        type=int,
        default=DEFAULT_MANAGEMENT_PORT,
        help=f"管理 WS 端口（默认 {DEFAULT_MANAGEMENT_PORT}）",
    )
    parser.add_argument(
        "--port",
        metavar="PATH",
        help="要打开的串口路径（如 COM3、/dev/ttyUSB0）",
    )
    parser.add_argument(
        "--baud",
        type=int,
        default=DEFAULT_BAUD,
        help=f"波特率（默认 {DEFAULT_BAUD}）",
    )
    parser.add_argument(
        "--hex",
        action="store_true",
        help="十六进制模式：stdin 按 HEX 解析写入，stdout 按 HEX 打印",
    )
    parser.add_argument(
        "--listen",
        action="store_true",
        help="只收不发：仅把串口数据转存到 stdout，不读取 stdin",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="列出当前串口并退出",
    )
    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.list:
        try:
            asyncio.run(_list_ports(args))
        except (CliError, OSError) as exc:
            sys.stderr.write(f"错误：{exc}\n")
            return 1
        return 0

    if not args.port:
        parser.error("缺少 --port：打开串口需指定 --port，或使用 --list 列出端口")

    state: dict = {"session_id": None}
    try:
        asyncio.run(_run(args, state))
    except KeyboardInterrupt:
        sys.stderr.write("\n收到 Ctrl-C，正在关闭…\n")
        if state["session_id"]:
            try:
                asyncio.run(_close_session(args.management_port, state["session_id"]))
            except Exception as exc:  # 会话可能已关闭，忽略并继续。
                sys.stderr.write(f"关闭会话失败：{exc}\n")
        return 130
    except (CliError, OSError) as exc:
        sys.stderr.write(f"错误：{exc}\n")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
