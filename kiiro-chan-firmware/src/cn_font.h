// 中文点阵字体渲染器
// HZK16 (GB2312 16x16) + ASC16 (8x16)，字库嵌入固件 flash。
// UTF-8 输入 -> 查 Unicode→GB2312 映射表(二分) -> 点阵绘制。
#pragma once
#include <TFT_eSPI.h>

namespace cnfont {

// 行高固定 16px；ASCII 宽 8px，汉字宽 16px。
static constexpr int LINE_H = 16;

// 在 (x,y) 绘制一行 UTF-8 字符串（不透明背景），返回绘制像素宽度。
int drawString(TFT_eSPI& gfx, int x, int y, const char* utf8, uint16_t fg, uint16_t bg);

// 自动换行绘制：超过 maxW 像素折行。从第 skipLines 行开始显示，最多绘制
// maxLines 行（窗口外不绘制但继续计数）。返回文本折行后的总行数。
int drawWrapped(TFT_eSPI& gfx, int x, int y, int maxW, int maxLines,
                const char* utf8, uint16_t fg, uint16_t bg, int skipLines = 0);

// 计算字符串像素宽度（不绘制）。
int textWidth(const char* utf8);

}  // namespace cnfont
