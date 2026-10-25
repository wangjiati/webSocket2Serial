#!/usr/bin/env bash
# WebSocket2Serial Linux 打包脚本。
#
# 在 ubuntu:18.04 容器内用 Rust 1.77.2 构建（保证 glibc 2.27 兼容），
# 并把二进制 / install.sh / README / 配置模板按固定目录结构打成 tar.gz：
#
#   websocket2serial/
#     websocket2serial        # 二进制（可执行）
#     install.sh              # 用户级安装脚本
#     README.md
#     config.toml.example
#
# 产物：dist/websocket2serial-linux-x86_64.tar.gz
#
# 无 Docker/Podman 时打印手动构建步骤并退出（退出码 1）。
#
# 用法：bash scripts/package-linux.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="ubuntu:18.04"
DIST="$ROOT/dist"
PKG_NAME="websocket2serial-linux-x86_64"
TARBALL="$DIST/$PKG_NAME.tar.gz"

# 选择容器运行时。
DOCKER=""
if command -v docker >/dev/null 2>&1; then
  DOCKER="docker"
elif command -v podman >/dev/null 2>&1; then
  DOCKER="podman"
fi

if [[ -z "$DOCKER" ]]; then
  echo "错误：未找到 docker / podman。" >&2
  echo "" >&2
  echo "请在有 Docker 的 Linux 构建机上执行本脚本；或手动在 Ubuntu 18.04 环境构建：" >&2
  cat >&2 <<'EOF'
  1. docker run --rm -it -v "$PWD":/src -w /src ubuntu:18.04 bash
  2. # ubuntu:18.04 已 EOL，先切 apt 源到 old-releases 再更新：
     sed -i \
       -e 's|archive\.ubuntu\.com|old-releases.ubuntu.com|g' \
       -e 's|security\.ubuntu\.com|old-releases.ubuntu.com|g' \
       -e 's|ports\.ubuntu\.com|old-releases.ubuntu.com|g' \
       /etc/apt/sources.list
     apt-get update && apt-get install -y build-essential pkg-config libudev-dev curl ca-certificates
  3. curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.77.2
     . "$HOME/.cargo/env"
  4. cargo +1.77.2 build --release
  5. 按本脚本末尾的 tar 步骤打包。
EOF
  echo "" >&2
  echo "注意：ubuntu:18.04 已 EOL，apt 源需切换到 old-releases.ubuntu.com。" >&2
  exit 1
fi

echo "使用 $DOCKER 在 $IMAGE 容器内构建……"
mkdir -p "$DIST"

"$DOCKER" run --rm \
  -v "$ROOT:/src:rw" \
  -w /src \
  "$IMAGE" \
  bash -c '
set -euo pipefail

# ubuntu:18.04 已 EOL：官方镜像 sources.list 仍指向 archive/security（会 404），
# 把 apt 源切到 old-releases.ubuntu.com 再更新（含 ports.ubuntu.com，如适用）。
switch_apt_source() {
  sed -i \
    -e "s|archive\.ubuntu\.com|old-releases.ubuntu.com|g" \
    -e "s|security\.ubuntu\.com|old-releases.ubuntu.com|g" \
    -e "s|ports\.ubuntu\.com|old-releases.ubuntu.com|g" \
    "$1"
}
switch_apt_source /etc/apt/sources.list
for f in /etc/apt/sources.list.d/*.list; do
  [ -e "$f" ] && switch_apt_source "$f"
done

apt-get update
apt-get install -y build-essential pkg-config libudev-dev curl ca-certificates
# 安装 Rust 1.77.2（rustup 非交互；--default-toolchain 已包含 1.77.2）。
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.77.2
fi
. "$HOME/.cargo/env"
cargo +1.77.2 build --release

# 打包（固定目录结构）。
STAGE="/src/target/package/linux/websocket2serial"
rm -rf "$STAGE"
mkdir -p "$STAGE" /src/dist
cp target/release/websocket2serial "$STAGE/websocket2serial"
cp packaging/install.sh "$STAGE/install.sh"
cp README.md "$STAGE/README.md"
cp packaging/config.toml.example "$STAGE/config.toml.example"
chmod +x "$STAGE/websocket2serial" "$STAGE/install.sh"
tar -C "$STAGE/.." -czf /src/dist/websocket2serial-linux-x86_64.tar.gz websocket2serial
echo "已生成 dist/websocket2serial-linux-x86_64.tar.gz"
'

echo "完成：$TARBALL"
