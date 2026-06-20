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

// Image-based desktop-pet renderer. Layers are transparent PNGs (pre-scaled to
// the screen, drawn 1:1). M5GFX can alpha-composite a PNG onto an rgb565 target
// but CRASHES onto argb8888, so we bake onto rgb565 sprites over the background:
//   spBody  = bg + hair_back + body + [blush] + costume + hair_front  (opaque)
//   faceComp[emotion] = copy(spBody) + face + [accessory]
// Per-emotion composites are cached. Dynamic changes (outfit/blush/glasses/char)
// mark things dirty and rebake (cheap: 200x240 PNGs decode fast).
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
static M5Canvas spBody(&M5.Display);   // rgb565, baked back layers over BG

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
static String faceEmotion[MAXFACE], faceFile[MAXFACE];
static M5Canvas faceComp[MAXFACE] = {
    M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display),
    M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display),
    M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display),
    M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display), M5Canvas(&M5.Display)};
static int8_t faceState[MAXFACE] = {0};
static int faceCount = 0;

static bool bodyReady = false;
static volatile bool g_needLoad = true;
static volatile bool g_savePref = false;
static float curBob = 0;
static inline float lerp(float a, float b, float t) { return a + (b - a) * t; }

static String baseName(const String& p) {
    int i = p.lastIndexOf('/');
    return i >= 0 ? p.substring(i + 1) : p;
}
static int catSlot(const String& name) {
    if (name.indexOf("hair_back") >= 0) return S_HAIRBACK;
    if (name.indexOf("hair_front") >= 0) return S_HAIRFRONT;
    if (name.indexOf("body") >= 0) return S_BODY;
    if (name.indexOf("blush") >= 0) return S_BLUSH;
    if (name.indexOf("costume") >= 0) return S_COSTUME;
    if (name.indexOf("accessory") >= 0) return S_ACCESSORY;
    return S_BG;
}
static bool readPngSize(const String& full, int* w, int* h) {
    File f = g_fs->open(full, FILE_READ);
    if (!f) return false;
    uint8_t b[24]; int n = f.read(b, 24); f.close();
    if (n < 24) return false;
    *w = (b[16] << 24) | (b[17] << 16) | (b[18] << 8) | b[19];
    *h = (b[20] << 24) | (b[21] << 16) | (b[22] << 8) | b[23];
    return (*w > 0 && *h > 0 && *w < 20000 && *h < 20000);
}
static bool makeRGB(M5Canvas& spr) {
    if (spr.getBuffer()) return true;
    spr.setColorDepth(16); spr.setPsram(true);
    return spr.createSprite(charW, charH);
}
static void drawInto(M5Canvas& spr, const String& file) {
    if (file.length()) spr.drawPngFile(*g_fs, (String(g_dir) + "/" + file).c_str(), 0, 0);
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
    File f = LittleFS.open(PREF_PATH, "w");
    if (f) { serializeJson(d, f); f.close(); }
}

static void scanAssets() {
    g_fs = nullptr;
    if (SD.exists(g_dir)) g_fs = &SD;
    else if (LittleFS.exists(g_dir)) g_fs = &LittleFS;
    if (!g_fs) return;
    for (int i = 0; i < S_COUNT; i++) { slotVarN[i] = 0; slotSel[i] = 0; }
    faceCount = 0;
    String firstAny;
    File dir = g_fs->open(g_dir);
    if (!dir) return;
    for (File e = dir.openNextFile(); e; e = dir.openNextFile()) {
        String name = baseName(String(e.name()));
        e.close();
        if (!(name.endsWith(".png") || name.endsWith(".PNG")) || name.startsWith(".")) continue;
        if (firstAny.length() == 0) firstAny = name;
        if (name.indexOf("face") >= 0) {
            if (faceCount < MAXFACE) {
                int fi = name.indexOf("face_");
                String emo = fi >= 0 ? name.substring(fi + 5) : "neutral";
                emo.replace(".png", ""); emo.replace(".PNG", "");
                faceEmotion[faceCount] = emo; faceFile[faceCount] = name; faceCount++;
            }
        } else {
            int s = catSlot(name);
            if (slotVarN[s] < 20) slotVars[s][slotVarN[s]++] = name;
        }
    }
    dir.close();
    if (firstAny.length() == 0) return;
    for (int s = 0; s < S_COUNT; s++)
        for (int a = 0; a < slotVarN[s]; a++)
            for (int b = a + 1; b < slotVarN[s]; b++)
                if (slotVars[s][b] < slotVars[s][a]) { String t = slotVars[s][a]; slotVars[s][a] = slotVars[s][b]; slotVars[s][b] = t; }
    int nw = 0, nh = 0;
    if (readPngSize(String(g_dir) + "/" + firstAny, &nw, &nh)) { charW = nw; charH = nh; }
    charX = (SCR_W - charW) / 2;
    charY = SCR_H - charH + 8;   // push down a few px so the breathing bob never lifts the bottom edge into view
    selectByToken(S_HAIRBACK, "short_black");
    selectByToken(S_HAIRFRONT, "short_black");
    selectByToken(S_COSTUME, "skirt");
    selectByToken(S_BLUSH, "faint");

    // restore persisted appearance if it's for this character
    if (g_pref.valid && g_pref.chr == charBasename()) {
        for (int s = 0; s < S_COUNT; s++) {
            if (g_pref.sel[s].length())
                for (int i = 0; i < slotVarN[s]; i++)
                    if (slotVars[s][i] == g_pref.sel[s]) { slotSel[s] = i; break; }
        }
        blushOn = g_pref.blush;
        accessoryOn = g_pref.glasses;
    }
}

static void invalidate() {
    bodyReady = false;
    for (int i = 0; i < MAXFACE; i++) if (faceState[i] == 1) faceState[i] = 0;
}

static void bakeBody() {
    if (!makeRGB(spBody)) return;
    spBody.fillScreen(spBody.color565((BGc >> 16) & 0xFF, (BGc >> 8) & 0xFF, BGc & 0xFF));
    drawInto(spBody, slotFile(S_BG));
    drawInto(spBody, slotFile(S_HAIRBACK));
    drawInto(spBody, slotFile(S_BODY));
    if (blushOn) drawInto(spBody, slotFile(S_BLUSH));
    drawInto(spBody, slotFile(S_COSTUME));
    drawInto(spBody, slotFile(S_HAIRFRONT));
    bodyReady = true;
}

static int findFace(const String& emo) {
    for (int i = 0; i < faceCount; i++) if (faceEmotion[i] == emo) return i;
    return -1;
}
static int findFaceAny(std::initializer_list<const char*> names) {
    for (auto n : names) { int i = findFace(String(n)); if (i >= 0) return i; }
    return -1;
}
static int ensureFace(int idx) {
    if (idx < 0) return -1;
    if (faceState[idx] == 1) return idx;
    if (faceState[idx] == -1) return -1;
    if (!makeRGB(faceComp[idx])) { faceState[idx] = -1; return -1; }
    spBody.pushSprite(&faceComp[idx], 0, 0);          // copy baked body
    drawInto(faceComp[idx], faceFile[idx]);            // face on top
    if (accessoryOn) drawInto(faceComp[idx], slotFile(S_ACCESSORY));
    faceState[idx] = 1;
    return idx;
}
static int moodFace() {
    switch (g_mood) {
        case Mood::Happy: return findFaceAny({"happy_1", "happy_2", "base"});
        case Mood::Sad: return findFaceAny({"sad_1", "sad_2", "base"});
        case Mood::Angry: return findFaceAny({"angry", "annoyed_1", "base"});
        case Mood::Surprised: return findFaceAny({"surprise", "scared", "base"});
        case Mood::Love: return findFaceAny({"happy_2", "smug", "happy_1", "base"});
        case Mood::Confused: return findFaceAny({"awkward", "annoyed_1", "base"});
        default: return -1;
    }
}
static int pickFace(bool blink, bool talkOpen) {
    int idx;
    if (g_asleep) idx = findFaceAny({"base_blink", "base", "neutral"});
    else if (g_speaking && talkOpen) idx = findFaceAny({"base_talk", "talk", "base", "neutral"});
    else if (g_speaking) idx = findFaceAny({"base", "neutral"});
    else if (g_mood != Mood::Neutral) { idx = moodFace(); if (idx < 0) idx = findFaceAny({"base", "neutral"}); }
    else if (blink) idx = findFaceAny({"base_blink", "blink", "base"});
    else idx = findFaceAny({"base", "neutral"});
    if (idx < 0 && faceCount > 0) idx = 0;
    return ensureFace(idx);
}

static void drawBar(int x, int y, int w, int h, int pct, uint16_t fg) {
    auto& g = canvas;
    g.fillRoundRect(x, y, w, h, h / 2, g.color565(0x33, 0x37, 0x44));
    int p = pct < 0 ? 0 : pct > 100 ? 100 : pct;
    int fw = (w - 4) * p / 100;
    if (fw >= h - 4) g.fillRoundRect(x + 2, y + 2, fw, h - 4, (h - 4) / 2, fg);
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
    drawBar(10, 30, 92, 7, s.energy, g.color565(0x6F, 0xD0, 0x86));
    drawBar(10, 43, 92, 7, s.bond, g.color565(0xFF, 0x8F, 0xA8));
    drawBar(10, 56, 92, 7, xpPct, g.color565(0x6F, 0xB7, 0xF0));
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
    if (slotSel[S_HAIRBACK] != oh || slotSel[S_HAIRFRONT] != ofr || slotSel[S_COSTUME] != oc ||
        slotSel[S_BLUSH] != ob || blushOn != obl || accessoryOn != og) {
        invalidate();
    }
}

static void resetAssets() {
    spBody.deleteSprite();
    for (int i = 0; i < MAXFACE; i++) { faceComp[i].deleteSprite(); faceState[i] = 0; }
    for (int i = 0; i < S_COUNT; i++) slotVarN[i] = 0;
    faceCount = 0;
    bodyReady = false;
}

// Fetch the gender's art set from the server into /pet/cache/<gender>/ (scaled to
// the screen height); only downloads files not already cached.
static void downloadAssets() {
    if (g_host[0] == 0 || WiFi.status() != WL_CONNECTED) return;
    String base = String("http://") + g_host + ":" + g_port;
    SD.mkdir("/pet");
    SD.mkdir("/pet/cache");
    String gdir = String("/pet/cache/") + g_gender;
    SD.mkdir(gdir.c_str());
    HTTPClient http;
    http.setConnectTimeout(5000);
    http.setTimeout(8000);
    http.begin(base + "/assets/" + g_gender);
    int code = http.GET();
    if (code != 200) { http.end(); return; }
    String body = http.getString();
    http.end();
    JsonDocument list;
    if (deserializeJson(list, body)) return;
    for (JsonVariant v : list.as<JsonArray>()) {
        String name = v.as<String>();
        if (!name.endsWith(".png")) continue;
        String path = gdir + "/" + name;
        if (SD.exists(path)) continue;
        String url = base + "/assets/" + g_gender + "/" + name + "?h=" + String(SCR_H);
        HTTPClient h2;
        h2.setConnectTimeout(5000);
        h2.setTimeout(8000);
        h2.begin(url);
        if (h2.GET() == 200) {
            File f = SD.open(path, FILE_WRITE);
            if (f) { h2.writeToStream(&f); f.close(); }
        }
        h2.end();
    }
}

static void renderFrame(uint32_t now) {
    if (g_needReset) { g_needReset = false; resetAssets(); }
    if (g_needDownload) {
        g_needDownload = false;
        canvas.fillScreen(canvas.color565((BGc >> 16) & 0xFF, (BGc >> 8) & 0xFF, BGc & 0xFF));
        canvas.setTextColor(canvas.color565(0x55, 0x5B, 0x6B));
        canvas.setFont(&fonts::Font2);
        canvas.setCursor(96, 110);
        canvas.print("loading art...");
        canvas.pushSprite(0, 0);
        downloadAssets();
        g_needLoad = true;
    }
    if (g_needLoad) {
        g_needLoad = false;
        scanAssets();
        if (slotVarN[S_BODY] == 0 && !g_downloadTried && WiFi.status() == WL_CONNECTED) {
            g_downloadTried = true;   // cache empty -> fetch from server, then reload
            g_needDownload = true;
        } else {
            if (g_pendingAppearance.length()) {
                applyAppearance(g_pendingAppearance.c_str());
                g_pendingAppearance = "";
            }
            invalidate();
        }
    }
    if (g_savePref) { g_savePref = false; savePref(); }
    bool showBlush = blushOn || g_mood == Mood::Love;
    static bool lastBlush = false;
    if (showBlush != lastBlush) { lastBlush = showBlush; blushOn = showBlush; invalidate(); }
    if (!bodyReady) { bakeBody(); for (int i = 0; i < MAXFACE; i++) if (faceState[i] == 1) faceState[i] = 0; }

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

    int f = pickFace(blinking, talkOpen);

    canvas.fillScreen(canvas.color565((BGc >> 16) & 0xFF, (BGc >> 8) & 0xFF, BGc & 0xFF));
    int y = charY + (int)curBob;
    if (f >= 0 && faceComp[f].getBuffer()) faceComp[f].pushSprite(&canvas, charX, y);
    else if (bodyReady) spBody.pushSprite(&canvas, charX, y);

    if (!bodyReady && f < 0) {
        canvas.setTextColor(canvas.color565(0x55, 0x5B, 0x6B));
        canvas.setFont(&fonts::Font4);
        canvas.setTextDatum(middle_center);
        canvas.drawString("loading...", SCR_W / 2, SCR_H / 2);
        canvas.setTextDatum(top_left);
    }
    drawHUD();
    canvas.pushSprite(0, 0);
}

static void renderTask(void*) {
    for (;;) { renderFrame(millis()); vTaskDelay(pdMS_TO_TICKS(45)); }
}

void init(const char* charId) {
    if (charId && *charId) snprintf(g_dir, sizeof g_dir, "/pet/%s", charId);
    loadPref();
    if (g_pref.valid && g_pref.chr.length()) snprintf(g_dir, sizeof g_dir, "/pet/%s", g_pref.chr.c_str());
    canvas.setColorDepth(16);
    canvas.setPsram(true);
    canvas.createSprite(SCR_W, SCR_H);
    xTaskCreatePinnedToCore(renderTask, "pet", 24576, nullptr, 1, nullptr, 0);
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
        applyAppearance(appearance);  // RAM-only (safe from this task)
        if (!bodyReady) {             // not loaded yet (e.g. cache was empty at boot) -> reload now
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
