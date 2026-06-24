#include "pet.h"
#include <FS.h>
#include <SD.h>
#include <LittleFS.h>
#include <M5Unified.h>
#include <math.h>
#include <initializer_list>
#include <ArduinoJson.h>
#include <HTTPClient.h>
#include <WiFi.h>

// Image-based desktop-pet renderer. The server pre-composites the character
// (RGB565+A8); the device holds it as one transparent layer and alpha-blends it
// by hand (M5GFX crashes alpha-compositing onto argb8888) over a fixed
// background, so the breathing bob moves only the character — no tearing.
// Composite + face layers are fetched on demand and cached on SD as KRA1 files.
namespace pet {

static volatile Mood g_mood = Mood::Neutral;
static volatile bool g_speaking = false, g_listening = false, g_thinking = false, g_asleep = false;
static volatile React g_react = React::None;
static volatile uint32_t g_react_start = 0;
static volatile int g_level = 1;

static constexpr int SCR_W = 320, SCR_H = 240;
static constexpr uint32_t BGc = 0xE9EFF1;
static char g_dir[40] = "/pet/cache/girl";   // SD cache (fetched from server); sync may switch gender
static fs::FS* g_fs = nullptr;
static char g_gender[12] = "girl";
static char g_host[40] = "";
static uint16_t g_port = 8080;
static volatile bool g_needDownload = false;
static volatile bool g_needReset = false;
static bool g_downloadTried = false;
static String g_pendingAppearance;

static int charW = 200, charH = 240, charX = 60, charY = 0;

static M5Canvas canvas(&M5.Display);
static M5Canvas sceneBg(&M5.Display);  // rgb565, full-screen scene background (fixed)
static char g_bg[28] = "";             // current scene bg name ("" = pastel)
static bool sceneActive = false;
static volatile bool g_needBg = false;

enum Slot { S_BG, S_HAIRBACK, S_BODY, S_BLUSH, S_COSTUME, S_HAIRFRONT, S_ACCESSORY, S_COUNT };
static String slotVars[S_COUNT][20];
static int slotVarN[S_COUNT] = {0};
static int slotSel[S_COUNT] = {0};

static bool blushOn = false;
static bool accessoryOn = true;

// persisted appearance preference (LittleFS), restored across reboots
static const char* PREF_PATH = "/petwear.json";
struct Pref { bool valid = false; String chr; String sel[S_COUNT]; bool blush = false; bool glasses = true; };
static Pref g_pref;

static String charBasename() {
    String c = g_dir; int i = c.lastIndexOf('/');
    return i >= 0 ? c.substring(i + 1) : c;
}

static constexpr int MAXFACE = 16;
static String faceEmotion[MAXFACE];
static int faceCount = 0;

static volatile bool g_needLoad = true;
static volatile bool g_savePref = false;
static float curBob = 0;
static inline float lerp(float a, float b, float t) { return a + (b - a) * t; }

static int catSlot(const String& name) {
    if (name.indexOf("hair_back") >= 0) return S_HAIRBACK;
    if (name.indexOf("hair_front") >= 0) return S_HAIRFRONT;
    if (name.indexOf("body") >= 0) return S_BODY;
    if (name.indexOf("blush") >= 0) return S_BLUSH;
    if (name.indexOf("costume") >= 0) return S_COSTUME;
    if (name.indexOf("accessory") >= 0) return S_ACCESSORY;
    return S_BG;
}
static String slotFile(int s) { return slotVarN[s] ? slotVars[s][slotSel[s]] : String(); }
static void selectByToken(int s, const String& token) {
    for (int i = 0; i < slotVarN[s]; i++)
        if (slotVars[s][i].indexOf(token) >= 0) { slotSel[s] = i; return; }
}

static void loadPref() {
    File f = LittleFS.open(PREF_PATH, "r");
    if (!f) return;
    JsonDocument d;
    if (deserializeJson(d, f)) { f.close(); return; }
    f.close();
    g_pref.valid = true;
    g_pref.chr = String((const char*)(d["char"] | ""));
    g_pref.sel[S_HAIRBACK] = String((const char*)(d["hairback"] | ""));
    g_pref.sel[S_HAIRFRONT] = String((const char*)(d["hairfront"] | ""));
    g_pref.sel[S_COSTUME] = String((const char*)(d["costume"] | ""));
    g_pref.sel[S_BLUSH] = String((const char*)(d["blushvar"] | ""));
    g_pref.blush = d["blush"] | false;
    g_pref.glasses = d["glasses"] | true;
    {
        const char* bg = d["bg"] | "";
        if (*bg) { strncpy(g_bg, bg, sizeof g_bg - 1); g_bg[sizeof g_bg - 1] = 0; g_needBg = true; }
    }
}

static void savePref() {
    JsonDocument d;
    d["char"] = charBasename();
    d["hairback"] = slotFile(S_HAIRBACK);
    d["hairfront"] = slotFile(S_HAIRFRONT);
    d["costume"] = slotFile(S_COSTUME);
    d["blushvar"] = slotFile(S_BLUSH);
    d["blush"] = blushOn;
    d["glasses"] = accessoryOn;
    d["bg"] = g_bg;
    File f = LittleFS.open(PREF_PATH, "w");
    if (f) { serializeJson(d, f); f.close(); }
}

static void invalidate() {}  // legacy no-op; new render path reloads via compositeKey change

// ============================================================================
// New render path: server pre-composites the character (RGB565+A8); device
// holds it as a single transparent layer and alpha-blends it over a fixed
// background, so the breathing bob no longer drags the background (no tearing).
// LovyanGFX crashes alpha-compositing onto argb8888, so we blend RGB565 by hand.
// ============================================================================

// Composited character layer (from /assets/composite); face layer cache below.
static uint16_t* charRGB = nullptr;
static uint8_t*  charA   = nullptr;
static bool      charReady = false;
static String    charKey;            // appearance signature currently loaded

static constexpr int FACE_SLOTS = 6; // LRU cache of decoded face layers
struct FaceBuf { String expr; uint16_t* rgb = nullptr; uint8_t* a = nullptr; uint32_t used = 0; bool valid = false; };
static FaceBuf faceBuf[FACE_SLOTS];

// Reusable scratch buffer for reading a whole KRA1 file off SD in one go.
static uint8_t* kraScratch = nullptr;
static constexpr size_t KRA_MAX = 8 + (size_t)320 * 240 * 3;

static bool allocLayer(uint16_t** rgb, uint8_t** a, int w, int h) {
    if (!*rgb) *rgb = (uint16_t*)heap_caps_malloc((size_t)w * h * 2, MALLOC_CAP_SPIRAM);
    if (!*a)   *a   = (uint8_t*)heap_caps_malloc((size_t)w * h, MALLOC_CAP_SPIRAM);
    return *rgb && *a;
}

// Manual RGB565 source-over blend of a (w*h) layer onto the canvas at (ox,oy).
// alpha==0 pixels are skipped; alpha==255 copied; else per-channel mix. Clipped.
static void blendLayer(int ox, int oy, const uint16_t* srgb, const uint8_t* sa, int w, int h) {
    uint16_t* dst = (uint16_t*)canvas.getBuffer();
    if (!dst || !srgb || !sa) return;
    for (int y = 0; y < h; y++) {
        int dy = oy + y;
        if (dy < 0 || dy >= SCR_H) continue;
        const uint16_t* sr = srgb + (size_t)y * w;
        const uint8_t*  sal = sa + (size_t)y * w;
        uint16_t* drow = dst + (size_t)dy * SCR_W;
        for (int x = 0; x < w; x++) {
            uint8_t al = sal[x];
            if (al == 0) continue;
            int dx = ox + x;
            if (dx < 0 || dx >= SCR_W) continue;
            uint16_t s = sr[x];                          // native RGB565
            if (al == 255) { drow[dx] = __builtin_bswap16(s); continue; }
            uint16_t d = __builtin_bswap16(drow[dx]);    // canvas sprite buffer is byte-swapped
            int ia = 255 - al;
            int sR = (s >> 11) & 0x1F, sG = (s >> 5) & 0x3F, sB = s & 0x1F;
            int dR = (d >> 11) & 0x1F, dG = (d >> 5) & 0x3F, dB = d & 0x1F;
            int oR = (sR * al + dR * ia) / 255;
            int oG = (sG * al + dG * ia) / 255;
            int oB = (sB * al + dB * ia) / 255;
            drow[dx] = __builtin_bswap16((uint16_t)((oR << 11) | (oG << 5) | oB));
        }
    }
}

// Ensure a KRA1 file is on SD (download from server if absent). Returns true if present.
static bool ensureKra(const String& url, const String& path) {
    if (SD.exists(path)) return true;
    if (g_host[0] == 0 || WiFi.status() != WL_CONNECTED) return false;
    HTTPClient h;
    h.setConnectTimeout(5000);
    h.setTimeout(30000);
    h.begin(url);
    bool ok = false;
    if (h.GET() == 200) {
        int len = h.getSize();   // content-length (-1 if chunked/unknown)
        File f = SD.open(path, FILE_WRITE);
        if (f) {
            int wrote = h.writeToStream(&f);
            f.close();
            ok = (wrote > 8) && (len < 0 || wrote == len) && SD.exists(path);
        }
    }
    h.end();
    if (!ok) SD.remove(path);   // never leave a partial/corrupt file cached
    return ok;
}

// Read a KRA1 file into rgb/a buffers. On success sets *w,*h from the header.
static bool readKra(const String& path, uint16_t* rgb, uint8_t* a, int* w, int* h) {
    File f = SD.open(path, FILE_READ);
    if (!f) return false;
    size_t fsz = f.size();
    if (fsz < 8 || fsz > KRA_MAX) { f.close(); return false; }
    if (!kraScratch) kraScratch = (uint8_t*)heap_caps_malloc(KRA_MAX, MALLOC_CAP_SPIRAM);
    if (!kraScratch) { f.close(); return false; }
    size_t got = f.read(kraScratch, fsz);
    f.close();
    if (got != fsz || memcmp(kraScratch, "KRA1", 4) != 0) return false;
    int dw = (kraScratch[4] << 8) | kraScratch[5];
    int dh = (kraScratch[6] << 8) | kraScratch[7];
    if (dw <= 0 || dh <= 0 || (size_t)(8 + dw * dh * 3) > fsz) return false;
    const uint8_t* p = kraScratch + 8;
    for (int i = 0; i < dw * dh; i++) { rgb[i] = (p[0] << 8) | p[1]; a[i] = p[2]; p += 3; }
    *w = dw; *h = dh;
    return true;
}

// Extract the variant token from the selected slot file, e.g.
// "10_hair_back_short_black.png" + "hair_back_" -> "short_black".
static String variantOf(int slot, const char* marker) {
    String fn = slotFile(slot);
    if (fn.length() == 0) return String();
    int i = fn.indexOf(marker);
    if (i < 0) return String();
    String v = fn.substring(i + strlen(marker));
    v.replace(".png", ""); v.replace(".PNG", "");
    return v;
}

// A signature of the current appearance; when it changes we reload the composite.
static String compositeKey() {
    String k = g_gender;
    k += "|" + variantOf(S_HAIRBACK, "hair_back_");
    k += "|" + variantOf(S_HAIRFRONT, "hair_front_");
    k += "|" + variantOf(S_COSTUME, "costume_");
    k += "|" + (blushOn ? variantOf(S_BLUSH, "blush_") : String());
    k += accessoryOn ? "|g" : "|";
    return k;
}

static String compositeURL() {
    String u = String("http://") + g_host + ":" + g_port + "/assets/composite/" + g_gender + "?h=" + SCR_H;
    String v;
    if ((v = variantOf(S_HAIRBACK, "hair_back_")).length())  u += "&hair_back=" + v;
    if ((v = variantOf(S_HAIRFRONT, "hair_front_")).length()) u += "&hair_front=" + v;
    if ((v = variantOf(S_COSTUME, "costume_")).length())      u += "&costume=" + v;
    if (blushOn && (v = variantOf(S_BLUSH, "blush_")).length()) u += "&blush=" + v;
    if (accessoryOn) u += "&glasses=1";
    return u;
}

static String kraDir() { return String("/pet/cache/kra"); }
static String compositePath() {
    String k = compositeKey();
    uint32_t hsh = 2166136261u;
    for (size_t i = 0; i < k.length(); i++) { hsh ^= (uint8_t)k[i]; hsh *= 16777619u; }
    char buf[56];
    snprintf(buf, sizeof buf, "/pet/cache/kra/comp_%08x.kra", hsh);
    return String(buf);
}

// (Re)load the character composite for the current appearance (cache on SD).
static void loadComposite() {
    // Don't build the character until the manifest is loaded (a costume is
    // selectable). Before that — e.g. offline at boot — appearance is incomplete
    // and the composite would be the bare body; show "loading..." instead.
    if (slotVarN[S_COSTUME] == 0) return;
    String key = compositeKey();
    if (charReady && key == charKey) return;
    SD.mkdir("/pet"); SD.mkdir("/pet/cache"); SD.mkdir("/pet/cache/kra");
    String path = compositePath();
    if (!ensureKra(compositeURL(), path)) return;   // keep old art if fetch fails
    if (!allocLayer(&charRGB, &charA, SCR_W, SCR_H)) return;
    int w = 0, hh = 0;
    bool rd = readKra(path, charRGB, charA, &w, &hh);
    if (rd) {
        charW = w; charH = hh;
        charX = (SCR_W - charW) / 2;
        charY = SCR_H - charH;            // bottom-align; full-amplitude bob is fine now
        charKey = key;
        charReady = true;
    } else {
        SD.remove(path);   // corrupt cache -> drop so next frame re-downloads
    }
}

// Get a face layer for `expr` (LRU cache; cache files on SD). Returns slot or -1.
static int loadFace(const String& expr) {
    if (expr.length() == 0) return -1;
    int lru = 0; uint32_t oldest = 0xFFFFFFFFu;
    for (int i = 0; i < FACE_SLOTS; i++) {
        if (faceBuf[i].valid && faceBuf[i].expr == expr) { faceBuf[i].used = millis(); return i; }
        if (faceBuf[i].used < oldest) { oldest = faceBuf[i].used; lru = i; }
    }
    FaceBuf& fb = faceBuf[lru];
    if (!allocLayer(&fb.rgb, &fb.a, SCR_W, SCR_H)) return -1;
    String path = kraDir() + "/face_" + g_gender + "_" + expr + ".kra";
    String url = String("http://") + g_host + ":" + g_port + "/assets/face/" + g_gender + "/" + expr + "?h=" + SCR_H;
    SD.mkdir("/pet/cache/kra");
    if (!ensureKra(url, path)) return -1;
    int w = 0, hh = 0;
    if (!readKra(path, fb.rgb, fb.a, &w, &hh)) { SD.remove(path); return -1; }
    fb.expr = expr; fb.used = millis(); fb.valid = true;
    return lru;
}

// Pull the asset name list from the server (names only, no image bytes) to fill
// the slot variants + face list, so appearance tokens can be resolved.
static void fetchManifest() {
    if (g_host[0] == 0 || WiFi.status() != WL_CONNECTED) return;
    String url = String("http://") + g_host + ":" + g_port + "/assets/" + g_gender;
    HTTPClient h;
    h.setConnectTimeout(5000); h.setTimeout(8000);
    h.begin(url);
    if (h.GET() != 200) { h.end(); return; }
    String body = h.getString();
    h.end();
    JsonDocument list;
    if (deserializeJson(list, body)) return;
    for (int i = 0; i < S_COUNT; i++) slotVarN[i] = 0;
    faceCount = 0;
    for (JsonVariant v : list.as<JsonArray>()) {
        String name = v.as<String>();
        if (!name.endsWith(".png")) continue;
        if (name.indexOf("face") >= 0) {
            if (faceCount < MAXFACE) {
                int fi = name.indexOf("face_");
                String emo = fi >= 0 ? name.substring(fi + 5) : String("neutral");
                emo.replace(".png", "");
                faceEmotion[faceCount] = emo; faceCount++;
            }
        } else {
            int s = catSlot(name);
            if (slotVarN[s] < 20) slotVars[s][slotVarN[s]++] = name;
        }
    }
    for (int s = 0; s < S_COUNT; s++)
        for (int a = 0; a < slotVarN[s]; a++)
            for (int b = a + 1; b < slotVarN[s]; b++)
                if (slotVars[s][b] < slotVars[s][a]) { String t = slotVars[s][a]; slotVars[s][a] = slotVars[s][b]; slotVars[s][b] = t; }
    selectByToken(S_HAIRBACK, "short_black");
    selectByToken(S_HAIRFRONT, "short_black");
    selectByToken(S_COSTUME, "skirt");
    selectByToken(S_BLUSH, "faint");
    if (g_pref.valid && g_pref.chr == charBasename()) {
        for (int s = 0; s < S_COUNT; s++)
            if (g_pref.sel[s].length())
                for (int i = 0; i < slotVarN[s]; i++)
                    if (slotVars[s][i] == g_pref.sel[s]) { slotSel[s] = i; break; }
        blushOn = g_pref.blush;
        accessoryOn = g_pref.glasses;
    }
}

static int findFace(const String& emo) {
    for (int i = 0; i < faceCount; i++) if (faceEmotion[i] == emo) return i;
    return -1;
}
static int findFaceAny(std::initializer_list<const char*> names) {
    for (auto n : names) { int i = findFace(String(n)); if (i >= 0) return i; }
    return -1;
}
static int moodFace(Mood m) {
    switch (m) {
        case Mood::Happy: return findFaceAny({"happy_1", "happy_2", "base"});
        case Mood::Sad: return findFaceAny({"sad_1", "sad_2", "base"});
        case Mood::Angry: return findFaceAny({"angry", "annoyed_1", "base"});
        case Mood::Surprised: return findFaceAny({"surprise", "scared", "base"});
        case Mood::Love: return findFaceAny({"happy_2", "smug", "happy_1", "base"});
        case Mood::Confused: return findFaceAny({"awkward", "annoyed_1", "base"});
        default: return -1;
    }
}
// Pick the desired expression NAME for the current state (the layer itself is
// fetched/decoded on demand by loadFace).
static String pickExpr(bool blink, bool talkOpen) {
    Mood m = g_mood;
    int idx;
    if (g_asleep) idx = findFaceAny({"base_blink", "base", "neutral"});
    else if (g_speaking && talkOpen) idx = findFaceAny({"base_talk", "talk", "base", "neutral"});
    else if (g_speaking) idx = findFaceAny({"base", "neutral"});
    else if (m != Mood::Neutral) { idx = moodFace(m); if (idx < 0) idx = findFaceAny({"base", "neutral"}); }
    else if (blink) idx = findFaceAny({"base_blink", "blink", "base"});
    else idx = findFaceAny({"base", "neutral"});
    if (idx < 0 && faceCount > 0) idx = 0;
    return idx >= 0 ? faceEmotion[idx] : String();
}

static void drawBar(int x, int y, int w, int h, int pct, uint16_t fg) {
    auto& g = canvas;
    g.fillRoundRect(x, y, w, h, h / 2, g.color565(0x33, 0x37, 0x44));
    int p = pct < 0 ? 0 : pct > 100 ? 100 : pct;
    int fw = (w - 4) * p / 100;
    if (fw >= h - 4) g.fillRoundRect(x + 2, y + 2, fw, h - 4, (h - 4) / 2, fg);
}
static void iconHeart(int cx, int cy, uint16_t c) {
    auto& g = canvas;
    g.fillCircle(cx - 2, cy - 1, 2, c);
    g.fillCircle(cx + 2, cy - 1, 2, c);
    g.fillTriangle(cx - 4, cy, cx + 4, cy, cx, cy + 4, c);
}
static void iconBolt(int cx, int cy, uint16_t c) {
    auto& g = canvas;   // a small lightning bolt
    g.fillTriangle(cx + 2, cy - 5, cx - 3, cy + 1, cx + 1, cy, c);
    g.fillTriangle(cx - 2, cy + 5, cx + 3, cy - 1, cx - 1, cy, c);
}
static void iconStar(int cx, int cy, uint16_t c) {
    auto& g = canvas;   // a 4-point sparkle
    g.fillTriangle(cx, cy - 5, cx - 2, cy, cx + 2, cy, c);
    g.fillTriangle(cx, cy + 5, cx - 2, cy, cx + 2, cy, c);
    g.fillTriangle(cx - 5, cy, cx, cy - 2, cx, cy + 2, c);
    g.fillTriangle(cx + 5, cy, cx, cy - 2, cx, cy + 2, c);
}
static void drawHUD() {
    auto& g = canvas;
    Stats s = getStats();
    g.fillRoundRect(4, 4, 104, 64, 8, g.color565(0x1C, 0x20, 0x2C));
    g.setTextColor(g.color565(0xFF, 0xCE, 0x5A));
    g.setFont(&fonts::Font2);
    g.setCursor(10, 8);
    g.printf("Lv%d", s.level);
    int xpPct = s.xpNeed > 0 ? s.xpInLevel * 100 / s.xpNeed : 0;
    uint16_t cEnergy = g.color565(0x6F, 0xD0, 0x86);
    uint16_t cBond = g.color565(0xFF, 0x8F, 0xA8);
    uint16_t cXp = g.color565(0x6F, 0xB7, 0xF0);
    iconBolt(14, 33, cEnergy);
    iconHeart(14, 46, cBond);
    iconStar(14, 59, cXp);
    drawBar(24, 30, 78, 7, s.energy, cEnergy);
    drawBar(24, 43, 78, 7, s.bond, cBond);
    drawBar(24, 56, 78, 7, xpPct, cXp);
}

static void applyAppearance(const char* json) {
    JsonDocument d;
    if (deserializeJson(d, json)) return;
    int oh = slotSel[S_HAIRBACK], ofr = slotSel[S_HAIRFRONT], oc = slotSel[S_COSTUME], ob = slotSel[S_BLUSH];
    bool obl = blushOn, og = accessoryOn;
    auto setf = [&](int s, const char* key) {
        const char* f = d[key] | "";
        if (*f)
            for (int i = 0; i < slotVarN[s]; i++)
                if (slotVars[s][i] == f) { slotSel[s] = i; break; }
    };
    setf(S_HAIRBACK, "hairback");
    setf(S_HAIRFRONT, "hairfront");
    setf(S_COSTUME, "costume");
    setf(S_BLUSH, "blushvar");
    if (!d["blush"].isNull()) blushOn = d["blush"];
    if (!d["glasses"].isNull()) accessoryOn = d["glasses"];
    if (d["bg"].is<const char*>()) setBg(d["bg"]);
    if (slotSel[S_HAIRBACK] != oh || slotSel[S_HAIRFRONT] != ofr || slotSel[S_COSTUME] != oc ||
        slotSel[S_BLUSH] != ob || blushOn != obl || accessoryOn != og) {
        invalidate();
    }
}

static void resetAssets() {
    for (int i = 0; i < S_COUNT; i++) slotVarN[i] = 0;
    faceCount = 0;
    charReady = false; charKey = "";
    for (int i = 0; i < FACE_SLOTS; i++) faceBuf[i].valid = false;
}

// Load the scene background (download from server if missing); sets sceneActive.
static void loadScene() {
    sceneActive = false;
    if (g_bg[0] == 0) { sceneBg.deleteSprite(); return; }
    String path = String("/pet/cache/bg/") + g_bg + ".png";
    bool online = g_host[0] && WiFi.status() == WL_CONNECTED;
    String url = String("http://") + g_host + ":" + g_port + "/assets/bg/" + g_bg + ".png?h=" + String(SCR_H);
    bool needDownload = !SD.exists(path);
    if (online && !needDownload) {
        // validate the cached file size against the server so edited/renamed bg refresh automatically
        HTTPClient hh;
        hh.setConnectTimeout(4000); hh.setTimeout(6000);
        hh.begin(url);
        int code = hh.sendRequest("HEAD");
        int remoteLen = hh.getSize();
        hh.end();
        if (code == 200 && remoteLen > 0) {
            File f = SD.open(path, FILE_READ);
            long localLen = f ? (long)f.size() : -1;
            if (f) f.close();
            if (localLen != remoteLen) needDownload = true;
        }
    }
    if (needDownload && online) {
        SD.mkdir("/pet"); SD.mkdir("/pet/cache"); SD.mkdir("/pet/cache/bg");
        HTTPClient h;
        h.setConnectTimeout(5000); h.setTimeout(8000);
        h.begin(url);
        if (h.GET() == 200) {
            File f = SD.open(path, FILE_WRITE);
            if (f) { h.writeToStream(&f); f.close(); }
        }
        h.end();
    }
    if (SD.exists(path)) {
        // read native size, scale to fill the whole screen (robust to any cached size)
        int w = SCR_W, h = SCR_H;
        File pf = SD.open(path, FILE_READ);
        if (pf) {
            uint8_t b[24];
            if (pf.read(b, 24) == 24) {
                int W = (b[16] << 24) | (b[17] << 16) | (b[18] << 8) | b[19];
                int H = (b[20] << 24) | (b[21] << 16) | (b[22] << 8) | b[23];
                if (W > 0 && H > 0 && W < 20000 && H < 20000) { w = W; h = H; }
            }
            pf.close();
        }
        if (!sceneBg.getBuffer()) { sceneBg.setColorDepth(16); sceneBg.setPsram(true); sceneBg.createSprite(SCR_W, SCR_H); }
        sceneBg.fillScreen(sceneBg.color565((BGc >> 16) & 0xFF, (BGc >> 8) & 0xFF, BGc & 0xFF));
        float sx = (float)SCR_W / w, sy = (float)SCR_H / h;
        sceneBg.drawPngFile(SD, path.c_str(), 0, 0, 0, 0, 0, 0, sx, sy);
        sceneActive = true;
    }
}

static void renderFrame(uint32_t now) {
    if (g_needReset) { g_needReset = false; resetAssets(); }
    if (g_needBg) { g_needBg = false; loadScene(); }
    if (g_needLoad) {
        g_needLoad = false;
        if (WiFi.status() == WL_CONNECTED) fetchManifest();   // asset names only (no image bytes)
        if (g_pendingAppearance.length()) {
            applyAppearance(g_pendingAppearance.c_str());
            g_pendingAppearance = "";
        }
        charReady = false;   // force composite (re)load below
    }
    if (g_savePref) { g_savePref = false; savePref(); }

    // Love mood implies blush; toggling blush changes the composited body.
    bool showBlush = blushOn || g_mood == Mood::Love;
    static bool lastBlush = false;
    if (showBlush != lastBlush) { lastBlush = showBlush; blushOn = showBlush; charReady = false; }

    // Reload the composite whenever the appearance changes (wear/blush/glasses);
    // cheap no-op when the key is unchanged.
    loadComposite();

    // Breathing bob — full amplitude now: the character is a transparent layer,
    // so the fixed background never moves with it (this is what kills the tearing).
    float t = now / 1000.0f;
    float bob = sinf(t * 2.2f) * 3.0f;
    React r = g_react;
    if (r != React::None) {
        float e = (now - g_react_start) / 1000.0f;
        if (e > 1.1f) g_react = React::None;
        else if (r == React::Hop || r == React::Startle) bob -= fabsf(sinf(e * PI * 2.2f)) * 26.0f;
    }
    curBob = lerp(curBob, bob, 0.4f);

    static uint32_t nextBlink = 0, blinkStart = 0; static bool blinking = false;
    if (!blinking && now > nextBlink) { blinking = true; blinkStart = now; }
    if (blinking && now - blinkStart > 130) { blinking = false; nextBlink = now + 1800 + (esp_random() % 3500); }
    bool talkOpen = ((now / 150) & 1) != 0;
    int fslot = loadFace(pickExpr(blinking, talkOpen));

    // Compose: fixed background, then transparent character + face at the bob offset.
    if (sceneActive && sceneBg.getBuffer()) {
        sceneBg.pushSprite(&canvas, 0, 0);
    } else {
        canvas.fillScreen(canvas.color565((BGc >> 16) & 0xFF, (BGc >> 8) & 0xFF, BGc & 0xFF));
    }
    if (charReady) {
        int y = charY + (int)curBob;
        blendLayer(charX, y, charRGB, charA, charW, charH);
        if (fslot >= 0 && faceBuf[fslot].valid)
            blendLayer(charX, y, faceBuf[fslot].rgb, faceBuf[fslot].a, charW, charH);
    } else {
        canvas.setTextColor(canvas.color565(0x55, 0x5B, 0x6B));
        canvas.setFont(&fonts::Font4);
        canvas.setTextDatum(middle_center);
        canvas.drawString("loading...", SCR_W / 2, SCR_H / 2);
        canvas.setTextDatum(top_left);
    }
    drawHUD();
    canvas.pushSprite(0, 0);
}

static TaskHandle_t renderHandle = nullptr;

// ---- Status overlay: drawn by THIS render task only. The LCD must be touched
// from a single task — letting main draw M5.Display concurrently corrupts the
// shared SPI bus (blank/garbled screen). main feeds the page via setStatusText()
// and toggles it with showStatus(); the render task owns all pixels.
static volatile bool g_showStatus = false;
static char g_statusText[640] = {0};

static void drawStatusPage() {
    auto& g = canvas;
    g.fillScreen(g.color565(0x0C, 0x0C, 0x18));
    g.setTextSize(1);
    g.setFont(&fonts::Font4);
    g.setTextColor(g.color565(0xFF, 0xD2, 0x4A));
    g.setCursor(8, 6);
    g.print("Status");
    g.setFont(&fonts::Font2);
    g.setTextColor(g.color565(0xDD, 0xDD, 0xEE));
    int y = 42; const int lh = 19;
    const char* p = g_statusText;
    char line[100];
    while (*p && y < SCR_H - 18) {
        int n = 0;
        while (*p && *p != '\n' && n < (int)sizeof(line) - 1) line[n++] = *p++;
        line[n] = 0;
        if (*p == '\n') p++;
        g.setCursor(8, y);
        g.print(line);
        y += lh;
    }
    g.setTextColor(g.color565(0x55, 0x66, 0x77));
    g.setCursor(8, SCR_H - 18);
    g.print("tap screen to return");
    g.pushSprite(0, 0);
}

static void renderTask(void*) {
    for (;;) {
        if (g_showStatus) { drawStatusPage(); vTaskDelay(pdMS_TO_TICKS(150)); }
        else { renderFrame(millis()); vTaskDelay(pdMS_TO_TICKS(45)); }
    }
}

void showStatus(bool on) { g_showStatus = on; }
void setStatusText(const char* t) {
    if (!t) return;
    strncpy(g_statusText, t, sizeof g_statusText - 1);
    g_statusText[sizeof g_statusText - 1] = 0;
}

void init(const char* charId) {
    if (charId && *charId) snprintf(g_dir, sizeof g_dir, "/pet/%s", charId);
    loadPref();
    if (g_pref.valid && g_pref.chr.length()) snprintf(g_dir, sizeof g_dir, "/pet/%s", g_pref.chr.c_str());
    canvas.setColorDepth(16);
    canvas.setPsram(true);
    canvas.createSprite(SCR_W, SCR_H);
    xTaskCreatePinnedToCore(renderTask, "pet", 24576, nullptr, 1, &renderHandle, 0);
}

void wear(const char* token) {
    if (!token || !*token) return;
    String t(token);
    for (int s = 0; s < S_COUNT; s++) selectByToken(s, t);
    invalidate();
    g_savePref = true;
}
void setBlush(bool on) { if (blushOn != on) { blushOn = on; invalidate(); g_savePref = true; } }
void setAccessory(bool on) { if (accessoryOn != on) { accessoryOn = on; invalidate(); g_savePref = true; } }

void setBg(const char* name) {
    const char* n = name ? name : "";
    if (strcmp(n, g_bg) == 0) return;
    strncpy(g_bg, n, sizeof g_bg - 1); g_bg[sizeof g_bg - 1] = 0;
    g_needBg = true;     // render task loads the scene + rebakes the character
    g_savePref = true;
}
void setCharacter(const char* id) {
    if (!id || !*id) return;
    snprintf(g_dir, sizeof g_dir, "/pet/%s", id);
    g_pref.valid = false;   // new character -> use its defaults until changed
    g_needReset = true;     // render task frees sprites
    g_needLoad = true;
    g_savePref = true;
}

void setServer(const char* host, uint16_t port) {
    if (host && *host) { strncpy(g_host, host, sizeof g_host - 1); g_host[sizeof g_host - 1] = 0; }
    g_port = port;
}

void applySync(const char* gender, const char* appearance) {
    if (gender && *gender) { strncpy(g_gender, gender, sizeof g_gender - 1); g_gender[sizeof g_gender - 1] = 0; }
    char want[40];
    snprintf(want, sizeof want, "/pet/cache/%s", g_gender);
    if (strcmp(g_dir, want) != 0) {
        strncpy(g_dir, want, sizeof g_dir - 1); g_dir[sizeof g_dir - 1] = 0;
        g_needReset = true;       // render task frees sprites
        g_downloadTried = false;  // allow a fresh download for the new gender
        g_pendingAppearance = (appearance && *appearance) ? String(appearance) : String();
        g_needLoad = true;
    } else if (appearance && *appearance) {
        applyAppearance(appearance);  // RAM-only (safe from this task); key change triggers reload
        if (!charReady) {             // not loaded yet (e.g. cache empty at boot) -> reload now
            g_pendingAppearance = appearance;
            g_downloadTried = false;
            g_needLoad = true;
        }
    }
}

String appearanceJson() {
    JsonDocument d;
    d["hairback"] = slotFile(S_HAIRBACK);
    d["hairfront"] = slotFile(S_HAIRFRONT);
    d["costume"] = slotFile(S_COSTUME);
    d["blushvar"] = slotFile(S_BLUSH);
    d["blush"] = blushOn;
    d["glasses"] = accessoryOn;
    d["bg"] = g_bg;
    String s;
    serializeJson(d, s);
    return s;
}

void setMood(Mood m) { g_mood = m; }
void setMoodByName(const char* e) {
    if (!e) return;
    String s(e);
    if (s == "happy") g_mood = Mood::Happy;
    else if (s == "sad") g_mood = Mood::Sad;
    else if (s == "angry") g_mood = Mood::Angry;
    else if (s == "surprised") g_mood = Mood::Surprised;
    else if (s == "love") g_mood = Mood::Love;
    else if (s == "confused") g_mood = Mood::Confused;
    else if (s == "sleepy") g_mood = Mood::Sleepy;
    else g_mood = Mood::Neutral;
}
void setSpeaking(bool v) { g_speaking = v; }
void setListening(bool v) { g_listening = v; }
void setThinking(bool v) { g_thinking = v; }
void setAsleep(bool v) { g_asleep = v; }
void react(React r) { g_react = r; g_react_start = millis(); }
void setLevel(int l) { g_level = l < 1 ? 1 : l; }

}  // namespace pet
