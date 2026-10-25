#!/usr/bin/env python3
"""WebSocket2Serial 浸泡/回环验收脚本（现场用）。

通过「管理 WS + 数据 WS」协议（与 examples/ws2serial-cli.py 相同，固定 127.0.0.1）
对 115200 串口做 TX-RX 回环：持续发送确定性的字节流，回环收到的字节逐字节比对，
统计收发计数、错配数与断线重连次数，用于验收「计数一致 / 长跑稳定 / 反复重连」。

依赖：Python 3.8+；实际运行时才需要 `websockets`（惰性 import，--help 无需安装）。

典型用法：
  # 30 分钟短跑（串口 TX-RX 短接）
  python scripts/soak.py --port COM3

  # 8 小时长跑，期间强制 50 次断线重连
  python scripts/soak.py --port COM3 --hours 8 --reconnects 50

  # 指定管理端口与波特率
  python scripts/soak.py --port /dev/ttyUSB0 --management-port 8765 --baud 115200

说明：
  - 发送内容为 0..255 循环字节流，绝对偏移 n 的期望字节为 n & 0xFF，O(1) 内存逐字节校验。
  - 回环要求 TX 与 RX 短接（或对端回显设备），否则收不到数据、脚本按错配/无数据退出。
  - 每次强制重连：干净关闭数据连接（1000）释放串口，再 session.open 重新接回，重复 N 次。
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
import time

DEFAULT_MANAGEMENT_PORT = 8765
DEFAULT_BAUD = 115200
DEFAULT_DURATION = 1800  # 默认 30 分钟（短跑）；--hours 可覆盖为更长。
DEFAULT_RECONNECTS = 0
DEFAULT_CHUNK = 512
DEFAULT_PAUSE = 0.05  # 512 字节 / 50ms ≈ 10 KB/s，低于 115200 8N1 线路速率，避免背压。
DEFAULT_SUMMARY_INTERVAL = 30.0

_id_counter = 0


def _next_id() -> str:
    global _id_counter
    _id_counter += 1
    return f"soak-{_id_counter}"


class SoakError(Exception):
    """可预期错误（打印后以非零码退出）。"""


def _import_websockets():
    try:
        import websockets
        from websockets.exceptions import ConnectionClosed, WebSocketException
    except ImportError as exc:  # pragma: no cover - 依赖环境
        raise SoakError(
            "缺少依赖 `websockets`，请先安装：python -m pip install websockets"
        ) from exc
    return websockets, ConnectionClosed, WebSocketException


def _connect(url):
    websockets, _, _ = _import_websockets()
    return websockets.connect(url)


async def _request(ws, action: str, params: dict):
    _, ConnectionClosed, _ = _import_websockets()
    req_id = _next_id()
    try:
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
                raise SoakError(
                    f"{error.get('code', 'UNKNOWN')}: {error.get('message', '未知错误')}"
                )
            # 事件（hello/ports.changed/...）忽略。
    except ConnectionClosed as exc:
        raise SoakError(f"管理连接已关闭（{action}）") from exc


async def _open_with_retry(mgmt, params: dict, attempts: int = 10) -> dict:
    """session.open，遇到 PORT_BUSY（旧连接尚未释放）时短暂退避重试。"""
    delay = 0.2
    for _ in range(attempts):
        try:
            return await _request(mgmt, "session.open", params)
        except SoakError as exc:
            if "PORT_BUSY" not in str(exc):
                raise
            await asyncio.sleep(delay)
            delay = min(delay * 2, 2.0)
    raise SoakError("session.open 反复 PORT_BUSY，旧连接未释放")


def _open_params(port: str, baud: int) -> dict:
    return {
        "path": port,
        "baudRate": baud,
        "dataBits": 8,
        "parity": "none",
        "stopBits": 1,
        "flowControl": "none",
    }


def _make_chunk(offset: int, size: int) -> bytes:
    """确定性字节流：绝对偏移 n 的字节为 n & 0xFF。"""
    return bytes((offset + i) & 0xFF for i in range(size))


def _verify(data_bytes: bytes, state: dict) -> None:
    for b in data_bytes:
        idx = state["recv"]
        if b != (idx & 0xFF):
            state["mismatches"] += 1
            if state["first_mismatch"] is None:
                state["first_mismatch"] = idx
        state["recv"] += 1


def _summary(state: dict) -> None:
    elapsed = time.monotonic() - state["started"]
    rate = state["recv"] / elapsed if elapsed > 0 else 0.0
    sys.stderr.write(
        f"[soak] 已发送 {state['sent']} B / 已接收 {state['recv']} B / "
        f"错配 {state['mismatches']} / 重连 {state['reconnects']} / "
        f"{rate:.0f} B/s\n"
    )
    sys.stderr.flush()


async def _counting_loop(data, args, state, end_at: float) -> None:
    """发送确定性字节流并校验回环，直到 end_at（monotonic 秒）。"""
    _, _, WebSocketException = _import_websockets()
    last_summary = time.monotonic()
    while time.monotonic() < end_at:
        try:
            now = time.monotonic()
            if now - state["last_send"] >= args.pause:
                chunk = _make_chunk(state["sent"], args.chunk)
                await data.send(chunk)
                state["sent"] += len(chunk)
                state["last_send"] = now
            try:
                msg = await asyncio.wait_for(data.recv(), timeout=0.05)
            except asyncio.TimeoutError:
                msg = None
            if isinstance(msg, (bytes, bytearray)):
                _verify(bytes(msg), state)
            # 文本帧（不应出现）忽略。
        except WebSocketException as exc:
            # 设备拔插 / 服务关闭导致的意外断连：转 SoakError 走可控报告路径。
            raise SoakError(f"数据连接意外断开：{exc}") from exc
        if now - last_summary >= args.summary_interval:
            _summary(state)
            last_summary = now


async def _run(args, state: dict) -> None:
    mgmt_url = f"ws://127.0.0.1:{args.management_port}/ws"
    params = _open_params(args.port, args.baud)

    duration = args.hours * 3600.0 if args.hours is not None else args.duration
    deadline = time.monotonic() + duration
    state["started"] = time.monotonic()

    # 重连间隔：reconnects 次均匀分布在总时长内（0 次则只跑到最后）。
    reconnect_interval = duration / (args.reconnects + 1) if args.reconnects else duration

    async with _connect(mgmt_url) as mgmt:
        phase_end = time.monotonic() + reconnect_interval
        while time.monotonic() < deadline:
            result = await _open_with_retry(mgmt, params)
            data_url = result.get("dataUrl")
            if not data_url:
                raise SoakError("session.open 未返回 dataUrl")
            sys.stderr.write(
                f"[soak] 会话 {result.get('sessionId')}（reused={result.get('reused')}）\n"
            )
            async with _connect(data_url) as data:
                await _counting_loop(data, args, state, min(deadline, phase_end))
            # 计数循环结束：仍在总时长内则计入一次重连并进入下一阶段。
            if time.monotonic() >= deadline:
                break
            state["reconnects"] += 1
            phase_end = min(deadline, time.monotonic() + reconnect_interval)

    # 结束校验。
    if state["sent"] == 0:
        raise SoakError("未发送任何数据")
    if state["recv"] == 0:
        raise SoakError("未收到任何回环数据（请确认 TX-RX 已短接或对端回显）")
    if state["mismatches"]:
        raise SoakError(
            f"回环错配 {state['mismatches']} 处（首个错配偏移 {state['first_mismatch']}）"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="soak.py",
        description="WebSocket2Serial 115200 回环浸泡/重连验收脚本。",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--port", required=True, help="串口路径（COM3、/dev/ttyUSB0 等）")
    parser.add_argument(
        "--management-port", type=int, default=DEFAULT_MANAGEMENT_PORT,
        help=f"管理 WS 端口（默认 {DEFAULT_MANAGEMENT_PORT}）",
    )
    parser.add_argument("--baud", type=int, default=DEFAULT_BAUD, help=f"波特率（默认 {DEFAULT_BAUD}）")
    parser.add_argument(
        "--duration", type=float, default=DEFAULT_DURATION,
        help=f"总时长秒数（默认 {DEFAULT_DURATION}，即 30 分钟短跑）",
    )
    parser.add_argument(
        "--hours", type=float, default=None,
        help="总时长小时数，覆盖 --duration（如 --hours 8）",
    )
    parser.add_argument(
        "--reconnects", type=int, default=DEFAULT_RECONNECTS,
        help=f"总时长内强制断线重连次数（默认 {DEFAULT_RECONNECTS}）",
    )
    parser.add_argument("--chunk", type=int, default=DEFAULT_CHUNK, help="每次发送字节数")
    parser.add_argument("--pause", type=float, default=DEFAULT_PAUSE, help="发送间隔秒数")
    parser.add_argument(
        "--summary-interval", type=float, default=DEFAULT_SUMMARY_INTERVAL,
        help="进度打印间隔秒数",
    )
    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.duration <= 0 or (args.hours is not None and args.hours <= 0):
        parser.error("时长必须为正数")
    if args.reconnects < 0:
        parser.error("--reconnects 不能为负")
    if args.chunk <= 0:
        parser.error("--chunk 必须为正数")
    if args.pause <= 0:
        parser.error("--pause 必须为正数")
    state: dict = {
        "sent": 0,
        "recv": 0,
        "mismatches": 0,
        "first_mismatch": None,
        "reconnects": 0,
        "last_send": 0.0,
        "started": 0.0,
    }
    try:
        asyncio.run(_run(args, state))
    except KeyboardInterrupt:
        sys.stderr.write("\n收到 Ctrl-C，正在结束…\n")
        _summary(state)
        return 130
    except (SoakError, OSError) as exc:
        _summary(state)
        sys.stderr.write(f"错误：{exc}\n")
        return 1
    _summary(state)
    sys.stderr.write("回环校验通过：收发字节一致，无错配。\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
