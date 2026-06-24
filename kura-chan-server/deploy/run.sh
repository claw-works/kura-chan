#!/usr/bin/env bash
# 启动 Kura-chan 服务端（开发模式：每次增量编译 debug 二进制）。
# 启动前会停掉上一次启动的实例(按 server.pid)，并把新进程 pid 写入 server.pid。
# 后台启动：  cd kura-chan-server && nohup bash deploy/run.sh >> nohup.log 2>&1 &
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PID_FILE="$ROOT/server.pid"

# 停掉上一次的实例（先按保存的 pid，再按二进制名兜底）
if [ -f "$PID_FILE" ]; then
  OLD="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [ -n "${OLD:-}" ] && kill -0 "$OLD" 2>/dev/null; then
    echo "[run] stopping old server (pid $OLD) ..."
    kill "$OLD" 2>/dev/null || true
  fi
fi
pkill -f "target/debug/kura-chan-server" 2>/dev/null || true
sleep 1

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

# 开发模式：每次启动都增量编译 debug 二进制（快），保证跑的是最新代码。
echo "[run] building (debug) ..."
cargo build
BIN="$ROOT/target/debug/kura-chan-server"

echo "[run] starting $BIN (listen ${KURA_SERVER_PORT:-8080}, AWS_REGION=${AWS_REGION})"
# exec 替换进程映像但保留 pid，所以写入的就是 server 进程的 pid
echo $$ > "$PID_FILE"
exec "$BIN"
