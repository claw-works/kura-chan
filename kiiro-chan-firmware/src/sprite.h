// 角色立绘：从 kura-chan-server 合成端点拉取 KRA1 (RGB565BE+A8)，
// 常驻内存，空闲界面绘制（黑底 alpha 混合）。
#pragma once
#include <TFT_eSPI.h>

namespace sprite {

// HTTP 拉取立绘（阻塞，开机时调用一次）。h 为目标像素高度。
bool fetch(const char* host, uint16_t port, const char* gender, int h);

bool loaded();
int width();
int height();

// 以 (x,y) 为左上角绘制，透明像素画黑（用于黑色背景）。
void draw(TFT_eSPI& tft, int x, int y);

}  // namespace sprite
