#!/usr/bin/env bash
# WebSocket2Serial Linux 安装脚本（免 root 用户级安装）。
#
# 功能：
#   1. 把二进制复制到 ~/.local/bin/websocket2serial（目标目录可用 --dest 覆盖）。
#   2. 生成绝对路径的 ~/.config/autostart/websocket2serial.desktop 自启项，
#      Exec 为二进制绝对路径（含空格等保留字符时按 Desktop Entry 规范转义）。
#   3. 可选立即启动（--start）。
#
# 用法：
#   ./install.sh [选项]
#
# 选项：
#   --source PATH    二进制路径（默认与本脚本同目录的 ./websocket2serial）
#   --dest DIR       安装目录（默认 ~/.local/bin）
#   --no-autostart   不生成开机自启 .desktop 项
#   --start          安装完成后立即启动
#   --help           显示帮助
#
# 说明：
#   - 依赖 $HOME；可用临时 HOME 做无副作用测试：HOME=$(mktemp -d) ./install.sh
#   - 自启项只负责「登录后启动进程」，不打开串口、不恢复会话。
#   - 应用每次启动若检测到自启已启用，会用自身 current_exe 刷新 Exec 路径。
#   - 卸载：删除 ~/.local/bin/websocket2serial 与
#     ~/.config/autostart/websocket2serial.desktop 即可。

set -euo pipefail

APP_NAME="websocket2serial"
SOURCE=""
DEST=""
AUTOSTART=1
START=0

usage() {
  cat <<'EOF'
用法：./install.sh [选项]

选项：
  --source PATH    二进制路径（默认与本脚本同目录的 ./websocket2serial）
  --dest DIR       安装目录（默认 ~/.local/bin）
  --no-autostart   不生成开机自启 .desktop 项
  --start          安装完成后立即启动
  --help           显示帮助
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      SOURCE="${2:-}"
      [[ -n "$SOURCE" ]] || { echo "错误：--source 缺少参数" >&2; exit 2; }
      shift 2
      ;;
    --dest)
      DEST="${2:-}"
      [[ -n "$DEST" ]] || { echo "错误：--dest 缺少参数" >&2; exit 2; }
      shift 2
      ;;
    --no-autostart)
      AUTOSTART=0
      shift
      ;;
    --start)
      START=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "错误：未知参数 $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "${HOME:-}" ]]; then
  echo "错误：未设置 HOME 环境变量" >&2
  exit 2
fi

# 默认来源：与本脚本同目录的二进制。
if [[ -z "$SOURCE" ]]; then
  SOURCE="$(cd "$(dirname "$0")" && pwd)/$APP_NAME"
fi
if [[ ! -f "$SOURCE" ]]; then
  echo "错误：找不到二进制 $SOURCE（可用 --source 指定）" >&2
  exit 1
fi

# 默认安装目录：~/.local/bin（绝对路径）。
if [[ -z "$DEST" ]]; then
  DEST="$HOME/.local/bin"
fi
# 归一化为绝对路径（相对 --dest 相对当前工作目录解析），并去掉尾部斜杠。
[[ "$DEST" = /* ]] || DEST="$(pwd)/$DEST"
DEST="${DEST%/}"
[[ -n "$DEST" ]] || DEST="/"

mkdir -p "$DEST"
# 安装二进制（install 保留可执行位）。
install -m 0755 "$SOURCE" "$DEST/$APP_NAME"
echo "已安装：$DEST/$APP_NAME"

# 二进制绝对路径（$DEST 已绝对）。
EXE_PATH="$DEST/$APP_NAME"

# Desktop Entry 的 Exec 参数引号/转义：整体加双引号，并在引号内对 " \ $ ` 前置
# 反斜杠（与应用的 desktop_entry 的转义规则一致；规范允许对无保留字符的路径加引号）。
desktop_exec_quote() {
  local arg="$1" out="" i ch
  for (( i = 0; i < ${#arg}; i++ )); do
    ch="${arg:i:1}"
    case "$ch" in
      '"'|'\'|'$'|'`') out+='\' ;;
    esac
    out+="$ch"
  done
  printf '"%s"' "$out"
}

if [[ "$AUTOSTART" == "1" ]]; then
  AUTOSTART_DIR="$HOME/.config/autostart"
  DESKTOP_FILE="$AUTOSTART_DIR/$APP_NAME.desktop"
  mkdir -p "$AUTOSTART_DIR"
  cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=WebSocket2Serial
Comment=WebSocket↔串口 本机常驻透传工具
Exec=$(desktop_exec_quote "$EXE_PATH")
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
  echo "已写入自启项：$DESKTOP_FILE"
fi

if [[ "$START" == "1" ]]; then
  echo "启动：$EXE_PATH"
  "$EXE_PATH" &
  echo "已请求启动（PID $!）；管理页默认 http://127.0.0.1:8766"
fi

echo "完成。管理 WS ws://127.0.0.1:8765/ws"
