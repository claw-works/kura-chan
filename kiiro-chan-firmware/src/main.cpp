// Kiiro-chan Firmware — 联网桌面助手
// 学而思"小喵掌机" (ESP32-WROVER-B + ST7735 128x160) 重刷固件
//
// 交互模型（B=返回，A=确认）:
//   详情视图(默认): 显示当前信息(带接收时间) + 右下角立绘; 上下滚动长文
//     └ B → 菜单
//   菜单: 最新信息 / 信息列表 / 任务列表 / 快捷语   (上下选择, A 确认, B 返回之前视图)
//   信息列表: 本机最近 10 条 (A 打开详情, B 返回菜单)
//   任务列表: 占位，后续接入 todo    (B 返回菜单)
//   快捷语: 发送 text_input 走 LLM  (A 发送, B 返回菜单)
// 消息存储: RAM 环形 10 条 + NVS 持久化（重启保留）
// 节能: WiFi modem sleep 保持 WS 长连接; CPU 80MHz; 20s 无操作熄屏(黑屏在线)

#include <Arduino.h>
#include <WiFi.h>
#include <time.h>
#include <TFT_eSPI.h>
#include <WebSocketsClient.h>
#include <ArduinoJson.h>
#include <Preferences.h>
#include "secrets.h"
#include "cn_font.h"
#include "sprite.h"

TFT_eSPI tft = TFT_eSPI();
WebSocketsClient ws;
Preferences prefs;

// 按键
static constexpr int PIN_KEY_UP    = 2;
static constexpr int PIN_KEY_DOWN  = 13;
static constexpr int PIN_KEY_LEFT  = 27;
static constexpr int PIN_KEY_RIGHT = 35; // input-only
static constexpr int PIN_KEY_A     = 34; // input-only
static constexpr int PIN_KEY_B     = 12; // strapping pin

static constexpr int PIN_BUZZER = 14;

static constexpr long GMT_OFFSET_SEC = 8 * 3600;
static constexpr int DST_OFFSET_SEC = 0;
static const char* NTP_SERVER = "ntp.aliyun.com";

static constexpr uint32_t SCREEN_OFF_AFTER_MS = 20000;

static bool time_synced = false;
static uint32_t last_activity_ms = 0;
static bool screen_dark = false;
static bool ws_ready = false;

// === 视图状态机 ===
enum class View : uint8_t { Detail, Menu, MsgList, TaskList, Quick };
static View view = View::Detail;

// === 消息存储 (0 = 最新) ===
struct Msg {
    String text;
    time_t ts;
};
static constexpr int MSG_MAX = 10;
static Msg msgs_store[MSG_MAX];
static int msg_count = 0;
static int msg_view_idx = 0;  // 详情视图当前显示哪条
static int list_sel = 0;      // 信息列表选中项

// === 菜单 ===
static const char* MENU_ITEMS[] = {"最新信息", "信息列表", "任务列表", "快捷语"};
static constexpr int NUM_MENU = 4;
static int menu_sel = 0;

// === 快捷语 ===
static const char* QUICK_PHRASES[] = {
    "早上好！",
    "我回来啦",
    "今天有什么安排？",
    "讲个笑话吧",
    "晚安",
};
static constexpr int NUM_PHRASES = sizeof(QUICK_PHRASES) / sizeof(QUICK_PHRASES[0]);
static int quick_sel = 0;

// === 气泡滚动 ===
static int bubble_scroll = 0;
static int bubble_total_lines = 0;
static constexpr int BUBBLE_VISIBLE_LINES = 6;

// === LLM 流式回复缓冲 ===
static String stream_text;
static bool streaming = false;

static bool render_dirty = false;   // WS 回调置位，主循环统一渲染
static bool arrived_beep = false;   // 新消息到达提示音（暗屏唤醒时）

static void beep(uint32_t freq_hz, uint32_t duration_ms) {
    ledcWriteTone(0, freq_hz);
    delay(duration_ms);
    ledcWriteTone(0, 0);
}

static void notify_beep() {
    beep(1500, 60); delay(30); beep(2200, 90);
}

// ================= 消息存储 =================

static void msgs_save() {
    prefs.begin("kiiro", false);
    prefs.putInt("count", msg_count);
    for (int i = 0; i < msg_count; i++) {
        char key[8];
        snprintf(key, sizeof(key), "m%d", i);
        prefs.putString(key, msgs_store[i].text.substring(0, 300));
        snprintf(key, sizeof(key), "t%d", i);
        prefs.putULong(key, (unsigned long)msgs_store[i].ts);
    }
    prefs.end();
}

static void msgs_load() {
    prefs.begin("kiiro", true);
    msg_count = prefs.getInt("count", 0);
    if (msg_count > MSG_MAX) msg_count = MSG_MAX;
    for (int i = 0; i < msg_count; i++) {
        char key[8];
        snprintf(key, sizeof(key), "m%d", i);
        msgs_store[i].text = prefs.getString(key, "");
        snprintf(key, sizeof(key), "t%d", i);
        msgs_store[i].ts = (time_t)prefs.getULong(key, 0);
    }
    prefs.end();
    Serial.printf("[Msgs] loaded %d from NVS\n", msg_count);
}

static void msgs_push(const String& text) {
    for (int i = min(msg_count, MSG_MAX - 1); i > 0; i--) {
        msgs_store[i] = msgs_store[i - 1];
    }
    msgs_store[0].text = text;
    msgs_store[0].ts = time_synced ? time(nullptr) : 0;
    if (msg_count < MSG_MAX) msg_count++;
    msg_view_idx = 0;
    msgs_save();
}

static String fmt_time(time_t ts, bool with_date) {
    if (ts == 0) return "";
    struct tm tmv;
    localtime_r(&ts, &tmv);
    char buf[20];
    if (with_date) snprintf(buf, sizeof(buf), "%02d-%02d %02d:%02d", tmv.tm_mon + 1, tmv.tm_mday, tmv.tm_hour, tmv.tm_min);
    else snprintf(buf, sizeof(buf), "%02d:%02d", tmv.tm_hour, tmv.tm_min);
    return String(buf);
}

// ================= 显示 =================
// 布局: 顶栏(0..16) | 主区域(18..128)

static String last_topbar_str;

static void draw_topbar() {
    struct tm timeinfo;
    char buf[32] = "--:--:--";
    if (time_synced && getLocalTime(&timeinfo, 50)) {
        static const char* WD[] = {"Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"};
        snprintf(buf, sizeof(buf), "%02d:%02d:%02d  %02d-%02d %s",
                  timeinfo.tm_hour, timeinfo.tm_min, timeinfo.tm_sec,
                  timeinfo.tm_mon + 1, timeinfo.tm_mday, WD[timeinfo.tm_wday]);
    }
    String s(buf);
    if (s != last_topbar_str) {
        tft.setTextColor(TFT_CYAN, TFT_BLACK);
        tft.setTextSize(1);
        tft.fillRect(0, 0, 144, 16, TFT_BLACK);
        tft.drawString(s, 4, 4);
        last_topbar_str = s;
    }
    tft.fillCircle(151, 8, 4, ws_ready ? TFT_GREEN : TFT_RED);
}

static void clear_main_area() {
    tft.fillRect(0, 18, 160, 110, TFT_BLACK);
}

static void draw_corner_sprite() {
    if (!sprite::loaded()) return;
    int x = 158 - sprite::width();
    int y = 128 - sprite::height();
    if (y < 18) y = 18;
    sprite::draw(tft, x, y);
}

// 当前详情气泡的完整内容（时间头 + 正文，或流式中的内容）
static String detail_content() {
    if (streaming) return stream_text.length() ? stream_text : "...";
    if (msg_count == 0) return "";
    const Msg& m = msgs_store[msg_view_idx];
    String head = fmt_time(m.ts, true);
    if (head.length()) return head + "\n" + m.text;
    return m.text;
}

// 气泡: 左侧白底黑字，6汉字/行 x 6行，可滚动；立绘先画、气泡叠其上
static void draw_bubble(const String& content) {
    const int bx = 2, by = 20, bw = 112, bh = 108;
    tft.fillRoundRect(bx, by, bw, bh, 6, TFT_WHITE);
    tft.drawRoundRect(bx, by, bw, bh, 6, TFT_DARKGREY);
    bubble_total_lines = cnfont::drawWrapped(tft, bx + 5, by + 4, bw - 16, BUBBLE_VISIBLE_LINES,
                                              content.c_str(), TFT_BLACK, TFT_WHITE,
                                              bubble_scroll);
    if (bubble_scroll > 0)
        tft.fillTriangle(bx + bw - 14, by + 10, bx + bw - 8, by + 10, bx + bw - 11, by + 4, TFT_DARKGREY);
    if (bubble_scroll + BUBBLE_VISIBLE_LINES < bubble_total_lines)
        tft.fillTriangle(bx + bw - 14, by + bh - 10, bx + bw - 8, by + bh - 10, bx + bw - 11, by + bh - 4, TFT_DARKGREY);
}

// 通用列表绘制（菜单/快捷语共用）: 深蓝底，黄色选中
static void draw_list(const char* const* items, int n, int sel, int visible) {
    const int bx = 2, by = 20, bw = 156, bh = 106;
    tft.fillRoundRect(bx, by, bw, bh, 6, TFT_NAVY);
    int first = max(0, min(sel - visible / 2, n - visible));
    for (int i = 0; i < visible && first + i < n; i++) {
        int idx = first + i;
        uint16_t fg = (idx == sel) ? TFT_YELLOW : TFT_LIGHTGREY;
        if (idx == sel) cnfont::drawString(tft, bx + 4, by + 6 + i * 19, ">", fg, TFT_NAVY);
        cnfont::drawString(tft, bx + 16, by + 6 + i * 19, items[idx], fg, TFT_NAVY);
    }
}

// 信息列表: 时间 + 截断正文
static void draw_msglist() {
    const int bx = 2, by = 20, bw = 156, bh = 106;
    tft.fillRoundRect(bx, by, bw, bh, 6, TFT_NAVY);
    if (msg_count == 0) {
        cnfont::drawString(tft, bx + 16, by + 40, "暂无信息", TFT_LIGHTGREY, TFT_NAVY);
        return;
    }
    static constexpr int VIS = 5;
    int first = max(0, min(list_sel - VIS / 2, msg_count - VIS));
    if (first < 0) first = 0;
    for (int i = 0; i < VIS && first + i < msg_count; i++) {
        int idx = first + i;
        uint16_t fg = (idx == list_sel) ? TFT_YELLOW : TFT_LIGHTGREY;
        if (idx == list_sel) cnfont::drawString(tft, bx + 2, by + 6 + i * 19, ">", fg, TFT_NAVY);
        String t = fmt_time(msgs_store[idx].ts, false);
        String row = (t.length() ? t + " " : "") + msgs_store[idx].text;
        // 截断到一行宽度(约140px)
        String clipped;
        int w = 0;
        for (size_t p = 0; p < row.length();) {
            uint8_t c = row[p];
            int cl = (c < 0x80) ? 1 : 3;
            int cw = (c < 0x80) ? 8 : 16;
            if (w + cw > 132) break;
            clipped += row.substring(p, p + cl);
            w += cw;
            p += cl;
        }
        cnfont::drawString(tft, bx + 12, by + 6 + i * 19, clipped.c_str(), fg, TFT_NAVY);
    }
}

// 统一渲染当前视图
static void render() {
    clear_main_area();
    switch (view) {
        case View::Detail: {
            draw_corner_sprite();
            String c = detail_content();
            if (c.length()) draw_bubble(c);
            else cnfont::drawString(tft, 6, 60, "暂无信息", TFT_DARKGREY, TFT_BLACK);
            break;
        }
        case View::Menu:
            draw_list(MENU_ITEMS, NUM_MENU, menu_sel, 5);
            break;
        case View::MsgList:
            draw_msglist();
            break;
        case View::TaskList: {
            const int bx = 2, by = 20;
            tft.fillRoundRect(bx, by, 156, 106, 6, TFT_NAVY);
            cnfont::drawString(tft, bx + 16, by + 30, "任务列表", TFT_LIGHTGREY, TFT_NAVY);
            cnfont::drawString(tft, bx + 16, by + 54, "(暂未接入)", TFT_DARKGREY, TFT_NAVY);
            break;
        }
        case View::Quick:
            draw_list(QUICK_PHRASES, NUM_PHRASES, quick_sel, 5);
            break;
    }
}

static void wake_screen() {
    if (screen_dark) {
        screen_dark = false;
        tft.fillScreen(TFT_BLACK);
        last_topbar_str = "";
        draw_topbar();
        render();
    }
    last_activity_ms = millis();
}

// ================= WebSocket =================

static void send_hello() {
    JsonDocument doc;
    doc["type"] = "hello";
    doc["device_id"] = KURA_DEVICE_ID;
    doc["firmware_version"] = "kiiro-0.2.0";
    JsonObject audio = doc["audio"].to<JsonObject>();
    audio["input_format"] = "none";
    audio["input_sample_rate"] = 0;
    audio["input_channels"] = 0;
    audio["input_frame_duration_ms"] = 0;
    audio["output_format"] = "none";
    audio["output_sample_rate"] = 0;
    audio["output_channels"] = 0;
    doc["capabilities"].to<JsonArray>();
    String json;
    serializeJson(doc, json);
    ws.sendTXT(json);
}

static void send_text_input(const char* text) {
    JsonDocument doc;
    doc["type"] = "text_input";
    doc["text"] = text;
    String json;
    serializeJson(doc, json);
    ws.sendTXT(json);
    Serial.printf("[WS] text_input: %s\n", text);
}

static void handle_server_json(JsonDocument& doc) {
    const char* type = doc["type"];
    if (!type) return;

    if (strcmp(type, "hello") == 0) {
        ws_ready = true;
        Serial.printf("[WS] session: %s\n", (const char*)doc["session_id"]);
        if (!screen_dark) draw_topbar();
    } else if (strcmp(type, "response") == 0) {
        // 服务端主动推送（定时提醒等）
        const char* text = doc["text"];
        if (text && *text) {
            msgs_push(String(text));
            bubble_scroll = 0;
            view = View::Detail;
            render_dirty = true;
            arrived_beep = true;
            Serial.printf("[WS] response: %s\n", text);
        }
    } else if (strcmp(type, "subtitle") == 0) {
        // 对话回复逐句流式
        const char* text = doc["text"];
        bool fin = doc["final"];
        if (text && *text) {
            if (!streaming) { streaming = true; stream_text = ""; }
            stream_text += text;
            bubble_scroll = 0;
            view = View::Detail;
            render_dirty = true;
        }
        if (fin && streaming) {
            streaming = false;
            if (stream_text.length()) msgs_push(stream_text);
            bubble_scroll = 0;
            render_dirty = true;
        }
    } else if (strcmp(type, "sync") == 0) {
        Serial.printf("[WS] sync: level=%d bond=%d energy=%d\n",
                      (int)doc["level"], (int)doc["bond"], (int)doc["energy"]);
    } else if (strcmp(type, "error") == 0) {
        Serial.printf("[WS] server error: %s\n", (const char*)doc["message"]);
    }
}

static void ws_event(WStype_t type, uint8_t* payload, size_t length) {
    switch (type) {
        case WStype_CONNECTED:
            Serial.println("[WS] Connected");
            send_hello();
            break;
        case WStype_DISCONNECTED:
            Serial.printf("[WS] Disconnected (wifi=%d)\n", WiFi.status());
            if (ws_ready) { ws_ready = false; if (!screen_dark) draw_topbar(); }
            break;
        case WStype_ERROR:
            Serial.printf("[WS] Error: %s\n", payload ? (char*)payload : "?");
            break;
        case WStype_TEXT: {
            JsonDocument doc;
            if (deserializeJson(doc, payload, length) == DeserializationError::Ok) {
                handle_server_json(doc);
            }
            break;
        }
        default: break;
    }
}

// ================= WiFi / NTP =================

static void connect_wifi() {
    WiFi.mode(WIFI_STA);
    static constexpr size_t NUM_CREDS = sizeof(WIFI_CREDS) / sizeof(WIFI_CREDS[0]);
    static constexpr uint32_t PER_AP_TIMEOUT_MS = 8000;

    for (size_t i = 0; i < NUM_CREDS; i++) {
        const auto& cred = WIFI_CREDS[i];
        Serial.printf("[WiFi] connecting to '%s' (%u/%u)...\n", cred.ssid, (unsigned)(i + 1), (unsigned)NUM_CREDS);
        tft.fillRect(0, 60, 160, 16, TFT_BLACK);
        cnfont::drawString(tft, 5, 60, (String("连接 ") + cred.ssid + " ...").c_str(), TFT_WHITE, TFT_BLACK);

        WiFi.begin(cred.ssid, cred.password);
        uint32_t start = millis();
        while (WiFi.status() != WL_CONNECTED && millis() - start < PER_AP_TIMEOUT_MS) {
            delay(300);
        }
        if (WiFi.status() == WL_CONNECTED) {
            Serial.printf("[WiFi] connected to '%s', IP=%s\n", cred.ssid, WiFi.localIP().toString().c_str());
            return;
        }
        Serial.printf("[WiFi] '%s' failed, trying next\n", cred.ssid);
        WiFi.disconnect(true);
        delay(100);
    }
    Serial.println("[WiFi] all networks failed");
    cnfont::drawString(tft, 5, 60, "WiFi 连接失败", TFT_RED, TFT_BLACK);
}

static void sync_ntp() {
    configTime(GMT_OFFSET_SEC, DST_OFFSET_SEC, NTP_SERVER);
    struct tm timeinfo;
    uint32_t start = millis();
    while (!getLocalTime(&timeinfo, 1000) && millis() - start < 10000) {}
    if (timeinfo.tm_year > 100) {
        time_synced = true;
        Serial.println("[NTP] synced");
    } else {
        Serial.println("[NTP] sync failed");
    }
}

// ================= 按键 =================

struct KeyDef { const char* name; int pin; bool input_only; };
static const KeyDef KEYS[] = {
    {"UP", PIN_KEY_UP, false},
    {"DOWN", PIN_KEY_DOWN, false},
    {"LEFT", PIN_KEY_LEFT, false},
    {"RIGHT", PIN_KEY_RIGHT, true},
    {"A", PIN_KEY_A, true},
    {"B", PIN_KEY_B, false},
};
static constexpr size_t NUM_KEYS = sizeof(KEYS) / sizeof(KEYS[0]);
static bool prev_state[NUM_KEYS] = {false};

static void on_key(const char* name) {
    beep(1800, 30);
    bool was_dark = screen_dark;
    wake_screen();
    if (was_dark) return; // 暗屏时按键只负责亮屏

    bool up = strcmp(name, "UP") == 0;
    bool down = strcmp(name, "DOWN") == 0;
    bool a = strcmp(name, "A") == 0;
    bool b = strcmp(name, "B") == 0;

    switch (view) {
        case View::Detail:
            if (up && bubble_scroll > 0) {
                bubble_scroll--;
                draw_bubble(detail_content());
            } else if (down && bubble_scroll + BUBBLE_VISIBLE_LINES < bubble_total_lines) {
                bubble_scroll++;
                draw_bubble(detail_content());
            } else if (b) {
                view = View::Menu;
                render();
            }
            break;

        case View::Menu:
            if (up)   { menu_sel = (menu_sel + NUM_MENU - 1) % NUM_MENU; render(); }
            else if (down) { menu_sel = (menu_sel + 1) % NUM_MENU; render(); }
            else if (a) {
                switch (menu_sel) {
                    case 0: view = View::Detail; render(); break; // 最新信息(回到最近查看)
                    case 1: view = View::MsgList; list_sel = 0; render(); break;
                    case 2: view = View::TaskList; render(); break;
                    case 3: view = View::Quick; render(); break;
                }
            } else if (b) { // 返回主页(信息详情)
                view = View::Detail;
                render();
            }
            break;

        case View::MsgList:
            if (up && list_sel > 0)   { list_sel--; render(); }
            else if (down && list_sel < msg_count - 1) { list_sel++; render(); }
            else if (a && msg_count > 0) {
                msg_view_idx = list_sel;
                bubble_scroll = 0;
                view = View::Detail;
                render();
            } else if (b) {
                view = View::Menu;
                render();
            }
            break;

        case View::TaskList:
            if (b) {
                view = View::Menu;
                render();
            }
            break;

        case View::Quick:
            if (up)   { quick_sel = (quick_sel + NUM_PHRASES - 1) % NUM_PHRASES; render(); }
            else if (down) { quick_sel = (quick_sel + 1) % NUM_PHRASES; render(); }
            else if (a) {
                if (ws_ready) {
                    send_text_input(QUICK_PHRASES[quick_sel]);
                    streaming = true;
                    stream_text = "";
                    bubble_scroll = 0;
                    view = View::Detail;
                    render();
                }
            } else if (b) {
                view = View::Menu;
                render();
            }
            break;
    }
}

// ================= 主流程 =================

void setup() {
    Serial.begin(115200);
    delay(200);
    Serial.println();
    Serial.println("=== Kiiro-chan Assistant boot ===");

    for (auto& k : KEYS) pinMode(k.pin, k.input_only ? INPUT : INPUT_PULLUP);
    ledcSetup(0, 2000, 10);
    ledcAttachPin(PIN_BUZZER, 0);

    tft.init();
    tft.setRotation(3);
    tft.setSwapBytes(true);
    tft.fillScreen(TFT_BLACK);

    msgs_load();

    WiFi.onEvent([](WiFiEvent_t ev, WiFiEventInfo_t info) {
        if (ev == ARDUINO_EVENT_WIFI_STA_DISCONNECTED)
            Serial.printf("[WiFi] disconnected, reason=%d\n", info.wifi_sta_disconnected.reason);
        else if (ev == ARDUINO_EVENT_WIFI_STA_GOT_IP)
            Serial.println("[WiFi] got IP (reconnected)");
    });
    connect_wifi();
    if (WiFi.status() == WL_CONNECTED) {
        WiFi.setAutoReconnect(true);
        sync_ntp();
        WiFi.setSleep(true);
        sprite::fetch(KURA_SERVER_HOST, KURA_SERVER_PORT, "girl", 56);
    }
    setCpuFrequencyMhz(80);
    Serial.printf("[Power] CPU %dMHz, WiFi modem sleep on\n", getCpuFrequencyMhz());

    String headers = String("Authorization: Bearer ") + KURA_API_KEY + "\r\n" +
                     "X-Device-Id: " + KURA_DEVICE_ID;
    ws.begin(KURA_SERVER_HOST, KURA_SERVER_PORT, "/ws/device");
    ws.setExtraHeaders(headers.c_str());
    ws.onEvent(ws_event);
    ws.setReconnectInterval(5000);
    ws.enableHeartbeat(25000, 5000, 2);

    // 开机进入详情视图，显示最新一条(NVS 恢复)
    tft.fillScreen(TFT_BLACK);
    last_topbar_str = "";
    draw_topbar();
    render();
    last_activity_ms = millis();
}

static uint32_t last_display_ms = 0;

void loop() {
    ws.loop();
    uint32_t now = millis();

    for (size_t i = 0; i < NUM_KEYS; i++) {
        bool pressed = digitalRead(KEYS[i].pin) == LOW;
        if (pressed != prev_state[i]) {
            prev_state[i] = pressed;
            if (pressed) on_key(KEYS[i].name);
        }
    }

    // WS 回调请求的渲染（新消息/流式更新）
    if (render_dirty) {
        render_dirty = false;
        bool was_dark = screen_dark;
        if (was_dark) {
            screen_dark = false;
            tft.fillScreen(TFT_BLACK);
            last_topbar_str = "";
            draw_topbar();
        }
        render();
        if (arrived_beep) {
            arrived_beep = false;
            if (was_dark) notify_beep();
            else beep(2000, 50);
        }
        last_activity_ms = millis();
    }

    if (!screen_dark && time_synced && now - last_display_ms >= 200) {
        last_display_ms = now;
        draw_topbar();
    }

    // === 连接看门狗 ===
    // WiFi 掉线超 30s: 强制重连（AutoReconnect 失效时的兜底）
    // WiFi 正常但 WS 超 90s 未 ready: 重建 WS 连接（库的自动重连有时卡死）
    {
        static uint32_t wifi_down_since = 0;
        static uint32_t ws_down_since = 0;
        if (WiFi.status() == WL_CONNECTED) {
            wifi_down_since = 0;
        } else {
            if (wifi_down_since == 0) wifi_down_since = now;
            if ((int32_t)(now - wifi_down_since) > 30000) {
                Serial.println("[Watchdog] WiFi down >30s, force reconnect");
                WiFi.disconnect();
                WiFi.reconnect();
                wifi_down_since = now;
            }
        }
        if (ws_ready || WiFi.status() != WL_CONNECTED) {
            ws_down_since = 0;
        } else {
            if (ws_down_since == 0) ws_down_since = now;
            if ((int32_t)(now - ws_down_since) > 90000) {
                Serial.println("[Watchdog] WS not ready >90s, rebuild connection");
                ws.disconnect();
                String headers = String("Authorization: Bearer ") + KURA_API_KEY + "\r\n" +
                                 "X-Device-Id: " + KURA_DEVICE_ID;
                ws.begin(KURA_SERVER_HOST, KURA_SERVER_PORT, "/ws/device");
                ws.setExtraHeaders(headers.c_str());
                ws.setReconnectInterval(5000);
                ws.enableHeartbeat(25000, 5000, 2);
                ws_down_since = now;
            }
        }
    }

    // 熄屏（黑屏保持 WS 在线）。带符号比较防无符号回绕。
    if (!screen_dark && (int32_t)(millis() - last_activity_ms) >= (int32_t)SCREEN_OFF_AFTER_MS) {
        screen_dark = true;
        tft.fillScreen(TFT_BLACK);
        Serial.println("[Screen] dark (WS stays online)");
    }
}
