// Kura-chan Firmware V0.1 - Face Display Test
// Draws a cute animated face on the StackChan LCD to verify hardware works.

#include <M5Unified.h>

// Screen dimensions (320x240 IPS)
static constexpr int SCREEN_W = 320;
static constexpr int SCREEN_H = 240;

// Face parameters
static constexpr int EYE_RADIUS = 28;
static constexpr int EYE_Y = 100;
static constexpr int LEFT_EYE_X = 110;
static constexpr int RIGHT_EYE_X = 210;
static constexpr int MOUTH_Y = 170;
static constexpr int MOUTH_W = 60;

// Animation state
static uint32_t last_blink_ms = 0;
static uint32_t blink_interval_ms = 3000;
static bool is_blinking = false;
static uint32_t blink_start_ms = 0;
static constexpr uint32_t BLINK_DURATION_MS = 150;

// Colors
static constexpr uint32_t BG_COLOR = 0x1A1A2E;      // Dark navy
static constexpr uint32_t EYE_COLOR = 0xFFFFFF;      // White
static constexpr uint32_t PUPIL_COLOR = 0x16213E;    // Dark blue
static constexpr uint32_t MOUTH_COLOR = 0xFF6B9D;    // Pink
static constexpr uint32_t CHEEK_COLOR = 0xFF9EBF;    // Light pink

void draw_face(bool blink) {
    auto& lcd = M5.Display;

    // Background
    lcd.fillScreen(BG_COLOR);

    if (blink) {
        // Closed eyes (horizontal lines)
        lcd.fillRoundRect(LEFT_EYE_X - EYE_RADIUS, EYE_Y - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
        lcd.fillRoundRect(RIGHT_EYE_X - EYE_RADIUS, EYE_Y - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
    } else {
        // Open eyes - white circles
        lcd.fillCircle(LEFT_EYE_X, EYE_Y, EYE_RADIUS, EYE_COLOR);
        lcd.fillCircle(RIGHT_EYE_X, EYE_Y, EYE_RADIUS, EYE_COLOR);

        // Pupils
        lcd.fillCircle(LEFT_EYE_X + 4, EYE_Y + 2, 14, PUPIL_COLOR);
        lcd.fillCircle(RIGHT_EYE_X + 4, EYE_Y + 2, 14, PUPIL_COLOR);

        // Eye highlights
        lcd.fillCircle(LEFT_EYE_X + 8, EYE_Y - 6, 6, EYE_COLOR);
        lcd.fillCircle(RIGHT_EYE_X + 8, EYE_Y - 6, 6, EYE_COLOR);
    }

    // Cheeks (blush)
    lcd.fillCircle(LEFT_EYE_X - 30, EYE_Y + 30, 12, CHEEK_COLOR);
    lcd.fillCircle(RIGHT_EYE_X + 30, EYE_Y + 30, 12, CHEEK_COLOR);

    // Smile - draw arc using small filled circle segments
    for (int i = -MOUTH_W / 2; i <= MOUTH_W / 2; i++) {
        int y_offset = (i * i) / 80; // Parabola for smile curve
        lcd.fillCircle(SCREEN_W / 2 + i, MOUTH_Y + y_offset, 3, MOUTH_COLOR);
    }

    // Title text
    lcd.setTextColor(0xCCCCCC);
    lcd.setTextSize(1);
    lcd.setFont(&fonts::Font2);
    lcd.setCursor(100, 220);
    lcd.print("Kura-chan v0.1");
}

void setup() {
    auto cfg = M5.config();
    cfg.internal_imu = false;  // Don't need IMU for this test
    cfg.internal_rtc = false;
    M5.begin(cfg);

    M5.Display.setRotation(1); // Landscape
    M5.Display.setBrightness(128);

    Serial.begin(115200);
    Serial.println("Kura-chan firmware v0.1 starting...");

    draw_face(false);
    Serial.println("Face drawn. Blinking every 3 seconds.");
}

void loop() {
    M5.update();

    uint32_t now = millis();

    // Blink logic
    if (!is_blinking && (now - last_blink_ms > blink_interval_ms)) {
        is_blinking = true;
        blink_start_ms = now;
        draw_face(true);
    }

    if (is_blinking && (now - blink_start_ms > BLINK_DURATION_MS)) {
        is_blinking = false;
        last_blink_ms = now;
        // Randomize next blink interval (2-5 seconds)
        blink_interval_ms = 2000 + (esp_random() % 3000);
        draw_face(false);
    }

    // Touch to change expression (simple test)
    if (M5.Touch.getCount() > 0) {
        auto t = M5.Touch.getDetail();
        if (t.wasPressed()) {
            Serial.printf("Touch at (%d, %d)\n", t.x, t.y);
            // Quick happy reaction - draw hearts
            M5.Display.setTextColor(MOUTH_COLOR);
            M5.Display.setTextSize(2);
            M5.Display.setCursor(140, 30);
            M5.Display.print("<3");
            delay(500);
            draw_face(false);
        }
    }

    delay(16); // ~60fps loop
}
