#!/usr/bin/env bash
# 启动 Kura-chan 服务端。读取 deploy/kura-server.env，从 server 根目录运行
# （服务端要在该目录读取 config/default.toml），监听 0.0.0.0:8080。
# 用法：bash deploy/run.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

ENV_FILE="${KURA_ENV_FILE:-$SCRIPT_DIR/kura-server.env}"
if [ ! -f "$ENV_FILE" ]; then
  echo "[run] 缺少 env 文件：$ENV_FILE" >&2
  echo "[run] 先执行：cp $SCRIPT_DIR/kura-server.env.example $SCRIPT_DIR/kura-server.env 并填入 VOLC_API_KEY" >&2
  exit 1
fi

# 载入环境变量
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

: "${VOLC_API_KEY:?[run] VOLC_API_KEY 未设置}"
: "${HARNESS_ARN:?[run] HARNESS_ARN 未设置}"
export AWS_REGION="${AWS_REGION:-us-west-2}"
export RUST_LOG="${RUST_LOG:-info,kura_chan_server=debug}"

BIN="$ROOT/target/release/kura-chan-server"
if [ ! -x "$BIN" ]; then
  echo "[run] 未找到 release 二进制，先编译 ..."
  cargo build --release
fi

echo "[run] 启动 $BIN （监听 0.0.0.0:8080，AWS_REGION=$AWS_REGION）"
exec "$BIN"
