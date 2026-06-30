#!/usr/bin/env bash
# Kura-chan 桌面端启动脚本。
# 配置读自 ~/.kura/.env（应用内 ⚙ 设置面板可修改并保存）。
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -f "$HOME/.kura/.env" ]; then
  echo "[kura-desktop] 提示: ~/.kura/.env 不存在 — 将用默认配置(连本地 127.0.0.1:18099)。"
  echo "               启动后在应用内 ⚙ 设置里填 server 地址 / api_key 并保存即可。"
fi

# 首次安装前端依赖
if [ ! -d node_modules ]; then
  echo "[kura-desktop] npm install ..."
  npm install
fi

# 前台启动（dev 模式：vite + Tauri 窗口）。Ctrl-C 或状态栏图标退出。
exec npm run tauri dev
