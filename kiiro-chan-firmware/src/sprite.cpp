#include "sprite.h"
#include <HTTPClient.h>

namespace sprite {

// KRA1: 8字节头 "KRA1" + w:u16BE + h:u16BE，体为每像素 RGB565BE + A8
static uint8_t* kra = nullptr;   // 合成后的立绘（含头，body+face 已叠好）
static int w_ = 0, h_ = 0;

bool loaded() { return kra != nullptr; }
int width() { return w_; }
int height() { return h_; }

static uint8_t* http_get(const String& url, int* out_len) {
    HTTPClient http;
    http.begin(url);
    http.setTimeout(10000);
    int code = http.GET();
    if (code != 200) {
        Serial.printf("[Sprite] HTTP %d: %s\n", code, url.c_str());
        http.end();
        return nullptr;
    }
    int len = http.getSize();
    if (len <= 8) { http.end(); return nullptr; }
    uint8_t* buf = (uint8_t*)malloc(len);
    if (!buf) { http.end(); return nullptr; }
    WiFiClient* stream = http.getStreamPtr();
    int got = 0;
    uint32_t start = millis();
    while (got < len && millis() - start < 10000) {
        int n = stream->read(buf + got, len - got);
        if (n > 0) got += n;
        else delay(10);
    }
    http.end();
    if (got != len || memcmp(buf, "KRA1", 4) != 0) { free(buf); return nullptr; }
    *out_len = len;
    return buf;
}

// face over body 一次性 alpha 合成（同尺寸画布，就地写入 body）
static void overlay(uint8_t* body, const uint8_t* face, int w, int h) {
    uint8_t* bp = body + 8;
    const uint8_t* fp = face + 8;
    for (size_t i = 0; i < (size_t)w * h; i++, bp += 3, fp += 3) {
        uint8_t fa = fp[2];
        if (fa == 0) continue;
        if (fa == 255) { bp[0] = fp[0]; bp[1] = fp[1]; bp[2] = 255; continue; }
        uint16_t fc = (fp[0] << 8) | fp[1];
        uint16_t bc = (bp[0] << 8) | bp[1];
        uint32_t r = (((fc >> 11) & 0x1F) * fa + ((bc >> 11) & 0x1F) * (255 - fa)) / 255;
        uint32_t g = (((fc >> 5) & 0x3F) * fa + ((bc >> 5) & 0x3F) * (255 - fa)) / 255;
        uint32_t b = ((fc & 0x1F) * fa + (bc & 0x1F) * (255 - fa)) / 255;
        uint16_t c = (r << 11) | (g << 5) | b;
        bp[0] = c >> 8;
        bp[1] = c & 0xFF;
        uint8_t ba = bp[2];
        bp[2] = fa + (uint16_t)ba * (255 - fa) / 255;
    }
}

bool fetch(const char* host, uint16_t port, const char* gender, int h) {
    String base = String("http://") + host + ":" + port + "/assets/";
    // 黄酱造型: 短粉发 + 白水手服
    String comp_url = base + "composite/" + gender +
        "?hair_back=short_pink&hair_front=short_pink&costume=seifuku_white&h=" + h;
    String face_url = base + "face/" + gender + "/base?h=" + h;

    int body_len = 0, face_len = 0;
    uint8_t* body = http_get(comp_url, &body_len);
    if (!body) return false;
    uint8_t* face = http_get(face_url, &face_len);

    int w = (body[4] << 8) | body[5];
    int hh = (body[6] << 8) | body[7];
    if (face) {
        int fw = (face[4] << 8) | face[5];
        int fh = (face[6] << 8) | face[7];
        if (fw == w && fh == hh) overlay(body, face, w, hh);
        else Serial.printf("[Sprite] face size mismatch %dx%d vs %dx%d\n", fw, fh, w, hh);
        free(face);
    }

    if (kra) free(kra);
    kra = body;
    w_ = w;
    h_ = hh;
    Serial.printf("[Sprite] loaded %dx%d (%d bytes, face=%s)\n", w_, h_, body_len, face ? "yes" : "no");
    return true;
}

void draw(TFT_eSPI& tft, int x, int y) {
    if (!kra) return;
    const uint8_t* p = kra + 8;
    static uint16_t line[160];
    int w = min(w_, 160);
    for (int row = 0; row < h_; row++) {
        const uint8_t* rp = p + (size_t)row * w_ * 3;
        for (int col = 0; col < w; col++) {
            uint16_t c = (rp[col * 3] << 8) | rp[col * 3 + 1];
            uint8_t a = rp[col * 3 + 2];
            // 黑底 alpha 混合: RGB565 各通道乘 a/255
            if (a == 0) { line[col] = TFT_BLACK; continue; }
            if (a == 255) { line[col] = c; continue; }
            uint32_t r = ((c >> 11) & 0x1F) * a / 255;
            uint32_t g = ((c >> 5) & 0x3F) * a / 255;
            uint32_t b = (c & 0x1F) * a / 255;
            line[col] = (r << 11) | (g << 5) | b;
        }
        tft.pushImage(x, y + row, w, 1, line);
    }
}

}  // namespace sprite
