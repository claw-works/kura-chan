#!/usr/bin/env bash
# 在目标小电脑（树莓派 / N100 等 Linux）上安装 Rust 并编译服务端。
# 用法：bash deploy/setup.sh
set -euo pipefail

# 定位 server 根目录（本脚本在 <root>/deploy/ 下）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

echo "[setup] server root: $ROOT"
uname -a

if [ ! -d patches/aws-runtime ]; then
  echo "[setup] 错误：缺少 patches/aws-runtime/，请把整个 kura-chan-server/ 完整拷贝过来" >&2
  exit 1
fi

# 安装 rustup（如缺）
if ! command -v cargo >/dev/null 2>&1; then
  echo "[setup] 未检测到 cargo，安装 rustup ..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

# edition 2024 需要 Rust >= 1.85，确保是最新 stable
rustup toolchain install stable >/dev/null 2>&1 || true
rustup default stable
rustc --version

# 安装编译依赖（ring / aws-lc-sys 需要 C 工具链、perl、cmake）
if command -v dnf >/dev/null 2>&1; then
  echo "[setup] 安装编译依赖（dnf: gcc gcc-c++ make perl pkgconfig openssl-devel cmake）..."
  sudo dnf install -y gcc gcc-c++ make perl pkgconfig openssl-devel cmake
elif command -v apt-get >/dev/null 2>&1; then
  echo "[setup] 安装编译依赖（apt: build-essential pkg-config libssl-dev perl cmake）..."
  sudo apt-get update -y
  sudo apt-get install -y build-essential pkg-config libssl-dev perl cmake
fi

echo "[setup] 编译 release ..."
cargo build --release

echo "[setup] 完成：$ROOT/target/release/kura-chan-server"
