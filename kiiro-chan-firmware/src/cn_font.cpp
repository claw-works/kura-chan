#include "cn_font.h"

// 嵌入的字库数据（platformio.ini board_build.embed_files）
extern const uint8_t HZK16[] asm("_binary_data_hzk16_bin_start");
extern const uint8_t ASC16[] asm("_binary_data_asc16_bin_start");
extern const uint8_t U2G[]   asm("_binary_data_u2g_bin_start");
extern const uint8_t U2G_END[] asm("_binary_data_u2g_bin_end");

namespace cnfont {

// --- UTF-8 解码：返回码点，*s 前进 ---
static uint32_t utf8_next(const char** s) {
    const uint8_t* p = (const uint8_t*)*s;
    uint32_t cp;
    int len;
    if (p[0] < 0x80) { cp = p[0]; len = 1; }
    else if ((p[0] & 0xE0) == 0xC0) { cp = p[0] & 0x1F; len = 2; }
    else if ((p[0] & 0xF0) == 0xE0) { cp = p[0] & 0x0F; len = 3; }
    else if ((p[0] & 0xF8) == 0xF0) { cp = p[0] & 0x07; len = 4; }
    else { (*s)++; return 0xFFFD; } // 非法字节，跳过
    for (int i = 1; i < len; i++) {
        if ((p[i] & 0xC0) != 0x80) { (*s) += i; return 0xFFFD; }
        cp = (cp << 6) | (p[i] & 0x3F);
    }
    (*s) += len;
    return cp;
}

// --- Unicode -> GB2312 二分查找。返回 0 表示未收录 ---
static uint16_t to_gb2312(uint32_t cp) {
    if (cp > 0xFFFF) return 0;
    struct Entry { uint16_t u, gb; }; // 小端，与生成脚本一致
    const Entry* table = (const Entry*)U2G;
    int lo = 0, hi = (int)((U2G_END - U2G) / sizeof(Entry)) - 1;
    while (lo <= hi) {
        int mid = (lo + hi) / 2;
        uint16_t u = table[mid].u;
        if (u == cp) return table[mid].gb;
        if (u < cp) lo = mid + 1;
        else hi = mid - 1;
    }
    return 0;
}

// --- 绘制一个 16x16 汉字点阵，返回宽度 16 ---
static void draw_cjk(TFT_eSPI& gfx, int x, int y, uint16_t gb, uint16_t fg, uint16_t bg) {
    uint8_t hi = gb >> 8, lo = gb & 0xFF;
    if (hi < 0xA1 || lo < 0xA1) return;
    size_t offset = ((size_t)(hi - 0xA1) * 94 + (lo - 0xA1)) * 32;
    uint16_t line[16];
    for (int row = 0; row < 16; row++) {
        uint16_t bits = (HZK16[offset + row * 2] << 8) | HZK16[offset + row * 2 + 1];
        for (int col = 0; col < 16; col++) {
            line[col] = (bits & (0x8000 >> col)) ? fg : bg;
        }
        gfx.pushImage(x, y + row, 16, 1, line);
    }
}

// --- 绘制一个 8x16 ASCII，返回宽度 8 ---
static void draw_ascii(TFT_eSPI& gfx, int x, int y, uint8_t ch, uint16_t fg, uint16_t bg) {
    size_t offset = (size_t)ch * 16;
    uint16_t line[8];
    for (int row = 0; row < 16; row++) {
        uint8_t bits = ASC16[offset + row];
        for (int col = 0; col < 8; col++) {
            line[col] = (bits & (0x80 >> col)) ? fg : bg;
        }
        gfx.pushImage(x, y + row, 8, 1, line);
    }
}

int drawString(TFT_eSPI& gfx, int x, int y, const char* utf8, uint16_t fg, uint16_t bg) {
    int cx = x;
    const char* s = utf8;
    while (*s) {
        uint32_t cp = utf8_next(&s);
        if (cp < 0x80) {
            draw_ascii(gfx, cx, y, (uint8_t)cp, fg, bg);
            cx += 8;
        } else {
            uint16_t gb = to_gb2312(cp);
            if (gb) {
                draw_cjk(gfx, cx, y, gb, fg, bg);
            } else {
                draw_ascii(gfx, cx, y, '?', fg, bg); // 未收录字符
                draw_ascii(gfx, cx + 8, y, '?', fg, bg);
            }
            cx += 16;
        }
    }
    return cx - x;
}

int drawWrapped(TFT_eSPI& gfx, int x, int y, int maxW, int maxLines,
                const char* utf8, uint16_t fg, uint16_t bg, int skipLines) {
    int cx = x, line = 0;
    const char* s = utf8;
    while (*s) {
        const char* prev = s;
        uint32_t cp = utf8_next(&s);
        if (cp == '\n') { cx = x; line++; continue; }
        int w = (cp < 0x80) ? 8 : 16;
        if (cx + w > x + maxW) { cx = x; line++; }
        // 只绘制滚动窗口内的行，窗口外仅计数
        int vis = line - skipLines;
        if (vis >= 0 && vis < maxLines) {
            s = prev;
            cp = utf8_next(&s);
            int dy = y + vis * LINE_H;
            if (cp < 0x80) {
                draw_ascii(gfx, cx, dy, (uint8_t)cp, fg, bg);
            } else {
                uint16_t gb = to_gb2312(cp);
                if (gb) draw_cjk(gfx, cx, dy, gb, fg, bg);
                else { draw_ascii(gfx, cx, dy, '?', fg, bg);
                       draw_ascii(gfx, cx + 8, dy, '?', fg, bg); }
            }
        }
        cx += w;
    }
    return line + 1;
}

int textWidth(const char* utf8) {
    int w = 0;
    const char* s = utf8;
    while (*s) {
        uint32_t cp = utf8_next(&s);
        w += (cp < 0x80) ? 8 : 16;
    }
    return w;
}

}  // namespace cnfont
