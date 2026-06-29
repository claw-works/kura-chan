#!/usr/bin/env bash
# Kura-chan 桌面端启动脚本：加载 env → 装依赖 → 起 Tauri dev。
# 用法：
#   cp kura-desktop.env.example kura-desktop.env   # 填 server 地址 + api_key
#   ./run.sh
set -euo pipefail
cd "$(dirname "$0")"

ENV_FILE="${ENV_FILE:-kura-desktop.env}"
if [ -f "$ENV_FILE" ]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
else
  echo "[kura-desktop] 未找到 $ENV_FILE，使用默认值（api_key 为空会连不上）。"
  echo "               cp kura-desktop.env.example $ENV_FILE 然后填写。"
fi

# 默认值（env 未设置时回退）
export KURA_WS_URL="${KURA_WS_URL:-ws://127.0.0.1:18099/ws/device}"
export KURA_HTTP_BASE="${KURA_HTTP_BASE:-http://127.0.0.1:18099}"
export KURA_DEVICE_ID="${KURA_DEVICE_ID:-KURA_DESKTOP_001}"
export KURA_API_KEY="${KURA_API_KEY:-}"

echo "[kura-desktop] WS=$KURA_WS_URL  HTTP=$KURA_HTTP_BASE  device=$KURA_DEVICE_ID  key=${KURA_API_KEY:+(set)}${KURA_API_KEY:-(empty)}"

# 首次安装前端依赖
if [ ! -d node_modules ]; then
  echo "[kura-desktop] npm install ..."
  npm install
fi

# 前台启动（dev 模式：vite 前端 + Tauri 窗口；Ctrl-C 退出）
exec npm run tauri dev
