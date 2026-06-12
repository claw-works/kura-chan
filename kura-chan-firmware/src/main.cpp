// Kura-chan Firmware V0.3 - NVS Config + Serial Commands
#include <M5Unified.h>
#include "config/config_store.h"
#include "config/serial_cmd.h"
#include "wifi/wifi_manager.h"
#include "ws/ws_client.h"

// === Face rendering (temporary - will be replaced by your UI design) ===
static constexpr int SCREEN_W = 320;
static constexpr int SCREEN_H = 240;
static constexpr int EYE_RADIUS = 28;
static constexpr int EYE_Y = 100;
static constexpr int LEFT_EYE_X = 110;
static constexpr int RIGHT_EYE_X = 210;
static constexpr int MOUTH_Y = 170;
static constexpr int MOUTH_W = 60;

static constexpr uint32_t BG_COLOR = 0x1A1A2E;
static constexpr uint32_t EYE_COLOR = 0xFFFFFF;
static constexpr uint32_t PUPIL_COLOR = 0x16213E;
static constexpr uint32_t MOUTH_COLOR = 0xFF6B9D;
static constexpr uint32_t CHEEK_COLOR = 0xFF9EBF;

// === State ===
static WifiManager wifi_mgr;
static WsClient ws_client;
static uint32_t last_blink_ms = 0;
static uint32_t blink_interval_ms = 3000;
static bool is_blinking = false;
static uint32_t blink_start_ms = 0;
static constexpr uint32_t BLINK_DURATION_MS = 150;
static String current_emotion = "neutral";
static bool face_dirty = true;

// === Status bar ===
void draw_status_bar() {
    auto& lcd = M5.Display;
    lcd.fillRect(0, 0, SCREEN_W, 16, 0x000000);
    lcd.setTextColor(0x888888);
    lcd.setFont(&fonts::Font0);
    lcd.setTextSize(1);

    // Wi-Fi status - direct check
    lcd.setCursor(4, 4);
    int wst = WiFi.status();
    if (wst == WL_CONNECTED) {
        lcd.printf("%s %s", WiFi.localIP().toString().c_str(), WiFi.SSID().c_str());
    } else {
        lcd.printf("WiFi:%d yiyi-pro", wst);
    }

    // Version + WS status
    lcd.setCursor(200, 4);
    lcd.print("v6 ");
    switch (ws_client.getState()) {
        case WsState::Disconnected: lcd.print("WS:off"); break;
        case WsState::Connecting:   lcd.print("WS:..."); break;
        case WsState::Connected:    lcd.print("WS:on");  break;
        case WsState::Ready:        lcd.print("WS:OK");  break;
    }
}

// === Face drawing ===
void draw_face(bool blink) {
    auto& lcd = M5.Display;
    lcd.fillRect(0, 16, SCREEN_W, SCREEN_H - 16, BG_COLOR);

    int face_offset_y = 16; // Below status bar

    if (blink) {
        lcd.fillRoundRect(LEFT_EYE_X - EYE_RADIUS, EYE_Y + face_offset_y - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
        lcd.fillRoundRect(RIGHT_EYE_X - EYE_RADIUS, EYE_Y + face_offset_y - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
    } else {
        lcd.fillCircle(LEFT_EYE_X, EYE_Y + face_offset_y, EYE_RADIUS, EYE_COLOR);
        lcd.fillCircle(RIGHT_EYE_X, EYE_Y + face_offset_y, EYE_RADIUS, EYE_COLOR);
        lcd.fillCircle(LEFT_EYE_X + 4, EYE_Y + face_offset_y + 2, 14, PUPIL_COLOR);
        lcd.fillCircle(RIGHT_EYE_X + 4, EYE_Y + face_offset_y + 2, 14, PUPIL_COLOR);
        lcd.fillCircle(LEFT_EYE_X + 8, EYE_Y + face_offset_y - 6, 6, EYE_COLOR);
        lcd.fillCircle(RIGHT_EYE_X + 8, EYE_Y + face_offset_y - 6, 6, EYE_COLOR);
    }

    // Cheeks
    lcd.fillCircle(LEFT_EYE_X - 30, EYE_Y + face_offset_y + 30, 12, CHEEK_COLOR);
    lcd.fillCircle(RIGHT_EYE_X + 30, EYE_Y + face_offset_y + 30, 12, CHEEK_COLOR);

    // Mouth
    for (int i = -MOUTH_W / 2; i <= MOUTH_W / 2; i++) {
        int y_offset = -(i * i) / 80 + 10;
        lcd.fillCircle(SCREEN_W / 2 + i, MOUTH_Y + face_offset_y + y_offset, 3, MOUTH_COLOR);
    }
}

// === WebSocket message handler ===
void on_ws_message(JsonDocument& doc) {
    const char* type = doc["type"];
    if (!type) return;

    if (strcmp(type, "state") == 0) {
        const char* state = doc["state"];
        Serial.printf("[App] State: %s\n", state);
    } else if (strcmp(type, "response") == 0) {
        const char* text = doc["text"];
        const char* emotion = doc["emotion"];
        Serial.printf("[App] Response: %s (emotion: %s)\n", text, emotion);
        if (emotion) {
            current_emotion = emotion;
            face_dirty = true;
        }
    } else if (strcmp(type, "tool_call") == 0) {
        const char* tool = doc["tool"];
        const char* call_id = doc["call_id"];
        Serial.printf("[App] Tool call: %s (id: %s)\n", tool, call_id);
        // TODO: execute tool and send result
        JsonDocument result;
        ws_client.sendToolResult(call_id, "ok", result);
    } else if (strcmp(type, "stt") == 0) {
        const char* text = doc["text"];
        bool is_final = doc["final"] | false;
        Serial.printf("[App] STT: %s (final: %d)\n", text, is_final);
    }
}

void on_ws_state_change(WsState state) {
    if (state == WsState::Connected) {
        ws_client.sendHello(
            configStore.getDeviceId().c_str(),
            "0.3.0"
        );
    }
    draw_status_bar();
}

// === Setup ===
void setup() {
    auto cfg = M5.config();
    M5.begin(cfg);
    M5.Display.setRotation(1);
    M5.Display.setBrightness(128);
    M5.Display.fillScreen(BG_COLOR);

    Serial.println("Kura-chan firmware v4 starting...");

    // Load persistent config
    configStore.begin();
    configStore.dump();

    // Init Wi-Fi - direct, no manager
    WiFi.mode(WIFI_STA);
    WiFi.setMinSecurity(WIFI_AUTH_WPA_PSK);
    delay(100);
    WiFi.begin("yiyi-pro", "99999999");
    delay(5000); // Wait for connection

    // Setup WebSocket callbacks
    ws_client.onJson(on_ws_message);
    ws_client.onStateChange(on_ws_state_change);

    // Draw initial face
    draw_status_bar();
    draw_face(false);

    Serial.println("[App] Setup complete, waiting for WiFi...");
}

// === Main loop ===
void loop() {
    M5.update();
    uint32_t now = millis();

    // Network stack (wifi_mgr disabled, using direct WiFi)

    // Start WS connection once Wi-Fi is up
    static bool ws_started = false;
    if (WiFi.status() == WL_CONNECTED && !ws_started) {
        ws_client.begin(
            configStore.getServerHost().c_str(),
            configStore.getServerPort(),
            configStore.getServerPath().c_str(),
            configStore.getApiKey().c_str(),
            configStore.getDeviceId().c_str()
        );
        ws_started = true;
        draw_status_bar();
    }
    if (WiFi.status() == WL_CONNECTED && ws_started) {
        ws_client.update();
    }
    if (WiFi.status() != WL_CONNECTED && ws_started) {
        ws_started = false;
        ws_client.disconnect();
        draw_status_bar();
    }

    // Serial commands
    serial_cmd_update();

    // Blink animation
    if (!is_blinking && (now - last_blink_ms > blink_interval_ms)) {
        is_blinking = true;
        blink_start_ms = now;
        draw_face(true);
    }
    if (is_blinking && (now - blink_start_ms > BLINK_DURATION_MS)) {
        is_blinking = false;
        last_blink_ms = now;
        blink_interval_ms = 2000 + (esp_random() % 3000);
        draw_face(false);
    }

    // Redraw face if emotion changed
    if (face_dirty) {
        draw_face(false);
        face_dirty = false;
    }

    // Update status bar every 2 seconds
    static uint32_t last_status_update = 0;
    if (now - last_status_update > 2000) {
        draw_status_bar();
        last_status_update = now;
    }

    delay(10);
}
