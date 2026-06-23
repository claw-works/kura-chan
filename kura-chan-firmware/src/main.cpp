// Kura-chan Firmware V0.4 - WS (canonical pattern) + face rendering.
// Built up from the known-good M5+WS debug build. No serial_cmd yet (added later
// once confirmed stable). WS rules: begin in setup, pump webSocket.loop() first in
// loop, send hello from CONNECTED event, keep drawing throttled.
#include <M5Unified.h>
#include <WiFi.h>
#include <SPI.h>
#include <SD.h>
#include <ArduinoJson.h>
#include <WebSocketsClient.h>
#include "config/config_store.h"
#include "SCSCL.h"
#include <math.h>
#include "pet/pet.h"

// ===================== Hardware: servo (SCS bus) + RGB LED (PY32) =====================
namespace hw {
static constexpr uint8_t PY32_ADDR = 0x6F;
static constexpr uint32_t PY32_FREQ = 100000;
static constexpr uint8_t R_M_L = 0x03, R_M_H = 0x04, R_O_L = 0x05, R_O_H = 0x06;
static constexpr uint8_t R_PU_L = 0x09, R_PU_H = 0x0A, R_PD_L = 0x0B, R_PD_H = 0x0C;
static constexpr uint8_t R_DRV_H = 0x14, R_LED_CFG = 0x24, R_LED_RAM = 0x30;

static SCSCL scs;
static bool servo_ok = false;
// servo geometry: ~0.293 deg/step. Centers found empirically per mounting.
static constexpr int CENTER_YAW = 470;    // forward (dead-straight)
static constexpr int CENTER_PITCH = 560;  // level == lowest safe (base below)
static constexpr float STEPS_PER_DEG = 1.0f / 0.293f;  // ~3.41
static constexpr int YAW_MAX = 80;        // ~±23deg left/right
static constexpr int PITCH_UP_MAX = 130;  // up only (~+38deg); down hits base

static bool w8(uint8_t r, uint8_t v) { return M5.In_I2C.writeRegister8(PY32_ADDR, r, v, PY32_FREQ); }
static uint8_t r8(uint8_t r) { return M5.In_I2C.readRegister8(PY32_ADDR, r, PY32_FREQ); }
static void setbit(uint8_t reg, uint8_t bit, bool on) {
    uint8_t v = r8(reg);
    if (on) v |= (1 << bit); else v &= ~(1 << bit);
    w8(reg, v);
}
static void pin_dir_out(uint8_t p) { setbit(p < 8 ? R_M_L : R_M_H, p < 8 ? p : p - 8, true); }
static void pin_pullup(uint8_t p) {
    setbit(p < 8 ? R_PU_L : R_PU_H, p < 8 ? p : p - 8, true);
    setbit(p < 8 ? R_PD_L : R_PD_H, p < 8 ? p : p - 8, false);
}
static void pin_write(uint8_t p, bool hi) { setbit(p < 8 ? R_O_L : R_O_H, p < 8 ? p : p - 8, hi); }

static void led(uint8_t r, uint8_t g, uint8_t b) {
    uint16_t c = ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
    uint8_t data[24];
    for (int i = 0; i < 12; i++) { data[i * 2] = c & 0xFF; data[i * 2 + 1] = (c >> 8) & 0xFF; }
    M5.In_I2C.writeRegister(PY32_ADDR, R_LED_RAM, data, 24, PY32_FREQ);
    w8(R_LED_CFG, r8(R_LED_CFG) | (1 << 6));
}

// gentle step move to an absolute raw position (never slams; soft-stalls)
static void smooth_to(uint8_t id, int target, int step_delay_ms) {
    int cur = scs.ReadPos(id);
    if (cur < 0 || cur > 1024) cur = target;
    int dir = (target >= cur) ? 1 : -1;
    for (int p = cur; (dir > 0) ? (p < target) : (p > target); p += dir * 4) {
        scs.WritePos(id, p, 20, 0);
        delay(step_delay_ms);
    }
    scs.WritePos(id, target, 20, 0);
}

// look at (yawDeg right+, pitchDeg up+) from forward, clamped & smoothed
static void look(int yawDeg, int pitchDeg, uint16_t time_ms) {
    if (!servo_ok) return;
    (void)time_ms;  // servo ignores large Time; smoothness comes from repeated 150ms updates
    int y = (int)(yawDeg * STEPS_PER_DEG);
    int p = (int)(pitchDeg * STEPS_PER_DEG);
    if (y > YAW_MAX) y = YAW_MAX; if (y < -YAW_MAX) y = -YAW_MAX;
    if (p < 0) p = 0; if (p > PITCH_UP_MAX) p = PITCH_UP_MAX;  // up only
    scs.WritePos(1, CENTER_YAW + y, 20, 0);
    scs.WritePos(2, CENTER_PITCH + p, 20, 0);  // +pitch = look up (higher raw)
}

static void init() {
    // servo power: expander pin0 output + pull-up + high
    pin_dir_out(0); pin_pullup(0); pin_write(0, true);
    delay(600);  // let servo MCU boot
    // RGB: pin13 output + pull-up + push-pull, 12 leds
    pin_dir_out(13); pin_pullup(13);
    setbit(R_DRV_H, 13 - 8, false);  // push-pull
    w8(R_LED_CFG, 12 & 0x3F);
    delay(150);
    led(0, 0, 0);
    // SCS servo bus
    scs.begin(UART_NUM_1, 1000000, 6, 7);
    delay(200);
    scs.EnableTorque(1, 1);
    scs.EnableTorque(2, 1);
    delay(50);
    // Ensure POSITION (servo) mode: if angle limits are 0/0 the servo is in
    // wheel/PWM mode and WritePos is ignored. Restore limits via EEPROM.
    for (int id = 1; id <= 2; id++) {
        int mn = scs.readWord(id, SCSCL_MIN_ANGLE_LIMIT_L);
        int mx = scs.readWord(id, SCSCL_MAX_ANGLE_LIMIT_L);
        if (mn < 0 || mx <= 0 || mn == mx) {
            scs.unLockEprom(id);
            scs.writeWord(id, SCSCL_MIN_ANGLE_LIMIT_L, 0);
            scs.writeWord(id, SCSCL_MAX_ANGLE_LIMIT_L, 1000);
            scs.LockEprom(id);
            delay(20);
        }
    }
    int p1 = scs.ReadPos(1);
    servo_ok = (p1 >= 0 && p1 <= 1024);
    // gentle move to neutral on boot
    if (servo_ok) { smooth_to(1, CENTER_YAW, 40); smooth_to(2, CENTER_PITCH, 40); }
}
}  // namespace hw

// === Face geometry ===
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

static WebSocketsClient webSocket;
static volatile int ws_state = 0;  // 0 off, 1 connected, 2 ready

// === Audio (PCM16 / 16kHz / mono) ===
static constexpr uint32_t SAMPLE_RATE = 16000;
static constexpr size_t REC_MAX_SAMPLES = SAMPLE_RATE * 30;   // up to 30s utterance
static constexpr size_t PLAY_MAX_BYTES = SAMPLE_RATE * 2 * 20; // up to 20s playback
static int16_t* rec_buf = nullptr;
static uint8_t* play_buf = nullptr;
static size_t rec_samples = 0;
static size_t play_bytes = 0;                 // total PCM bytes received for current reply
static size_t play_pos = 0;                   // bytes already fed to speaker
static volatile bool server_done = false;     // server sent speak_done
static volatile bool speaker_pending = false; // first audio arrived → switch to speaker
static constexpr size_t PLAY_PIECE = 16000;   // feed speaker in ~0.5s pieces
// Jitter buffer: how much PCM to accumulate before starting playback. The server
// is cross-Pacific from the device, so streamed TTS arrives with jitter; without
// a cushion the speaker queue underruns and audio stutters. ~1.5s @16k/16-bit.
// Loaded from config at boot (LittleFS /config.json). ~2s @16k/16-bit default.
static size_t PREBUFFER_BYTES = SAMPLE_RATE * 2 * 2;

enum class AudioState { Idle, Listening, Waiting, Speaking };
static AudioState audio_state = AudioState::Idle;
static uint32_t waiting_since = 0;

// === VAD (energy-based) auto-listen ===
static constexpr uint32_t VAD_THRESH = 1000;      // start threshold (begin speaking)
static constexpr uint32_t VAD_KEEP_THRESH = 1000; // silence = below this; >this keeps turn alive
static int VAD_MIN_RUN = 3;                       // consecutive loud chunks to confirm speech (config)
static constexpr int VAD_MIN_VOICED = 5;          // min total loud chunks (~0.5s) to actually send
static constexpr uint32_t SILENCE_END_MS = 2000;  // (unused in hold-to-talk) trailing silence
static constexpr uint32_t RELEASE_HOLD_MS = 300;  // head released this long -> submit (debounce)
static constexpr uint32_t MIN_HOLD_MS = 500;      // must hold at least this long to count (tap = ignore)
static uint32_t NO_SPEECH_MS = 6000;              // give up if no speech after a wake (config)
static constexpr uint32_t FOLLOWUP_NO_SPEECH_MS = 30000;  // post-answer follow-up window
// Adaptive (relative-to-ambient) VAD: floor tracks ambient noise; speech = energy
// rising clearly above the floor; submit after it falls back near the floor.
// These are runtime-tunable (loaded from /config.json at boot).
static float    VAD_RISE_FACTOR = 2.0f;  // speech start: energy > floor * this
static float    VAD_KEEP_FACTOR = 1.4f;  // still talking: energy > floor * this
static uint32_t VAD_MIN_MARGIN  = 150;   // and at least this much above floor
static uint32_t VAD_END_SILENCE_MS = 700;   // low for this long after speech -> send
static float noise_floor = 0;                      // adaptive ambient estimate (per turn)
static uint32_t listen_start_ms = 0;
static uint32_t baseline_until = 0;   // ambient-calibration window end (noise-robust VAD)
static uint32_t last_voice_ms = 0;
static uint32_t last_touch_ms = 0;  // hold-to-talk: last time head was touched
static bool speech_detected = false;
static int voiced_run = 0;
static int voiced_total = 0;
static bool followup_listen = false;
static size_t chunk_start = 0;
static bool chunk_pending = false;
static volatile uint32_t vad_level = 0;  // debug: last chunk mean-abs

// === Head touch (Si12T capacitive sensor on internal I2C @ 0x68) ===
static constexpr uint8_t SI12T_ADDR = 0x68;
static constexpr uint32_t SI12T_FREQ = 100000;
static bool head_touch_ok = false;
static volatile uint8_t head_raw = 0;  // debug: last raw OUTPUT1 byte

static void si12t_w(uint8_t reg, uint8_t val) {
    M5.In_I2C.writeRegister8(SI12T_ADDR, reg, val, SI12T_FREQ);
}

static void head_touch_init() {
    // enable all channels + reference calibration
    const uint8_t en[] = {0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F};
    for (uint8_t r : en) si12t_w(r, 0x00);
    // ctrl2: S/W reset + sleep enable, then run
    si12t_w(0x09, 0x0F);
    si12t_w(0x09, 0x07);
    // ctrl1: auto mode
    si12t_w(0x08, 0x22);
    // sensitivity LOW level 3 (0x33) on channels 1..5
    for (uint8_t r = 0x02; r <= 0x06; r++) si12t_w(r, 0x33);
    head_touch_ok = true;
}

static bool head_touched() {
    if (!head_touch_ok) return false;
    uint8_t v = M5.In_I2C.readRegister8(SI12T_ADDR, 0x10, SI12T_FREQ);
    head_raw = v;
    // 3 channels, 2 bits each
    return ((v >> 0) & 0x03) || ((v >> 2) & 0x03) || ((v >> 4) & 0x03);
}

static uint32_t last_blink_ms = 0;
static uint32_t blink_interval_ms = 3000;
static bool is_blinking = false;
static uint32_t blink_start_ms = 0;
static constexpr uint32_t BLINK_DURATION_MS = 150;
static String current_emotion = "neutral";
static uint32_t emotion_at = 0;
static bool face_dirty = false;

// === Idle sleep ===
static constexpr uint32_t SLEEP_AFTER_MS = 120000;  // idle (awake) time before sleeping; counts from last activity / speak-done
static constexpr uint8_t AWAKE_BRIGHTNESS = 128;
static constexpr uint8_t SLEEP_BRIGHTNESS = 6;
static uint32_t last_activity_ms = 0;
static bool asleep = false;

// === Manual control overrides (set by server 'control' msgs, cleared on sleep) ===
static bool man_led = false;
static uint8_t man_r = 0, man_g = 0, man_b = 0;
static bool man_yaw_set = false;
static int man_yaw_deg = 0;
static bool man_pitch_set = false;
static int man_pitch_deg = 0;            // up only (>=0); down hits base
static int gesture = 0;                  // 0 none, 1 nod, 2 shake (one-shot)
static uint32_t gesture_start = 0;
static constexpr uint32_t GESTURE_MS = 1300;
static int cur_volume_pct = 100;  // M5.Speaker volume (100% = 255) — loud for noisy venues

// === Second screen: tap the LCD to toggle a status page (independent of the
// head touch sensor, which is hold-to-talk). While in Status the pet render
// task is suspended and main owns the LCD. ===
enum class UiScreen : uint8_t { Pet, Status };
static UiScreen ui_screen = UiScreen::Pet;
static uint32_t status_last_draw = 0;

void draw_status_bar() {
    // No-op: the avatar render task owns the whole LCD now. Status/IP/WS/VAD
    // overlay can be re-added later via avatar.addTask() or a speech balloon.
    return;
    auto& lcd = M5.Display;
    lcd.fillRect(0, 0, SCREEN_W, 16, 0x000000);
    lcd.setTextColor(0x888888);
    lcd.setFont(&fonts::Font0);
    lcd.setTextSize(1);
    lcd.setCursor(4, 4);
    if (WiFi.status() == WL_CONNECTED) {
        lcd.printf("%s", WiFi.localIP().toString().c_str());
    } else {
        lcd.print("WiFi...");
    }
    lcd.setCursor(230, 4);
    lcd.print(ws_state == 2 ? "WS:OK" : ws_state == 1 ? "WS:on" : "WS:off");

    lcd.setCursor(120, 4);
    switch (audio_state) {
        case AudioState::Listening: lcd.setTextColor(0xFF5555); lcd.print("LIS"); break;
        case AudioState::Waiting:   lcd.setTextColor(0xFFFF55); lcd.print("THINK"); break;
        case AudioState::Speaking:  lcd.setTextColor(0x55FF55); lcd.print("SPK"); break;
        default:                    lcd.setTextColor(0x666666); lcd.print(asleep ? "zzz" : "idle"); break;
    }
}

// === Second screen: build the status page text. Rendering happens in the pet
// render task (single owner of the LCD); main only assembles the text here. ===
static void build_status_text(char* out, size_t cap) {
    bool wifi_ok = (WiFi.status() == WL_CONNECTED);
    int bat = M5.Power.getBatteryLevel();
    bool charging = ((int)M5.Power.isCharging() != 0);
    const char* st = asleep ? "sleep"
        : audio_state == AudioState::Listening ? "listen"
        : audio_state == AudioState::Waiting ? "think"
        : audio_state == AudioState::Speaking ? "speak" : "idle";
    pet::Stats s = pet::getStats();
    String ip = wifi_ok ? WiFi.localIP().toString() : String("-");
    String ssid = wifi_ok ? WiFi.SSID() : String("disconnected");
    snprintf(out, cap,
        "WiFi  %s\n"
        "IP    %s  %ddBm\n"
        "WS    %s\n"
        "Srv   %s:%u\n"
        "Batt  %d%%%s   Vol %d%%\n"
        "State %s   vad %u\n"
        "Pet   Lv%d  xp %d/%d\n"
        "Grow  bond %d  energy %d  turns %ld\n"
        "ID    %s",
        ssid.c_str(),
        ip.c_str(), wifi_ok ? (int)WiFi.RSSI() : 0,
        ws_state == 2 ? "ready" : ws_state == 1 ? "connected" : "offline",
        configStore.getServerHost().c_str(), configStore.getServerPort(),
        bat, charging ? "+" : "", cur_volume_pct,
        st, (unsigned)vad_level,
        s.level, s.xpInLevel, s.xpNeed,
        s.bond, s.energy, s.totalTurns,
        configStore.getDeviceId().c_str());
}

// === Face primitives ===
static void clear_face() {
    M5.Display.fillRect(0, 16, SCREEN_W, SCREEN_H - 16, BG_COLOR);
}
static void draw_cheeks() {
    auto& lcd = M5.Display; int oy = 16;
    lcd.fillCircle(LEFT_EYE_X - 30, EYE_Y + oy + 30, 12, CHEEK_COLOR);
    lcd.fillCircle(RIGHT_EYE_X + 30, EYE_Y + oy + 30, 12, CHEEK_COLOR);
}
// pupils offset (dx,dy); wide enlarges eyes; closed draws sleepy bars
static void draw_eyes(int dx, int dy, bool wide, bool closed) {
    auto& lcd = M5.Display; int oy = 16;
    int r = wide ? EYE_RADIUS + 5 : EYE_RADIUS;
    if (closed) {
        lcd.fillRoundRect(LEFT_EYE_X - EYE_RADIUS, EYE_Y + oy - 2, EYE_RADIUS * 2, 6, 3, 0x9999AA);
        lcd.fillRoundRect(RIGHT_EYE_X - EYE_RADIUS, EYE_Y + oy - 2, EYE_RADIUS * 2, 6, 3, 0x9999AA);
        return;
    }
    lcd.fillCircle(LEFT_EYE_X, EYE_Y + oy, r, EYE_COLOR);
    lcd.fillCircle(RIGHT_EYE_X, EYE_Y + oy, r, EYE_COLOR);
    lcd.fillCircle(LEFT_EYE_X + dx, EYE_Y + oy + dy, 14, PUPIL_COLOR);
    lcd.fillCircle(RIGHT_EYE_X + dx, EYE_Y + oy + dy, 14, PUPIL_COLOR);
    lcd.fillCircle(LEFT_EYE_X + dx + 4, EYE_Y + oy + dy - 6, 6, EYE_COLOR);
    lcd.fillCircle(RIGHT_EYE_X + dx + 4, EYE_Y + oy + dy - 6, 6, EYE_COLOR);
}
static void draw_blink_eyes() {
    auto& lcd = M5.Display; int oy = 16;
    lcd.fillRoundRect(LEFT_EYE_X - EYE_RADIUS, EYE_Y + oy - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
    lcd.fillRoundRect(RIGHT_EYE_X - EYE_RADIUS, EYE_Y + oy - 3, EYE_RADIUS * 2, 6, 3, EYE_COLOR);
}
static constexpr int MOUTH_CX = SCREEN_W / 2;
static constexpr int MOUTH_OY = MOUTH_Y + 16;
static void clear_mouth() {
    M5.Display.fillRect(MOUTH_CX - 45, MOUTH_OY - 28, 90, 56, BG_COLOR);
}
static void mouth_smile() {
    auto& lcd = M5.Display;
    for (int i = -MOUTH_W / 2; i <= MOUTH_W / 2; i++) {
        int yo = -(i * i) / 80 + 10;
        lcd.fillCircle(MOUTH_CX + i, MOUTH_OY + yo, 3, MOUTH_COLOR);
    }
}
static void mouth_o() {  // small open round mouth (listening)
    M5.Display.fillCircle(MOUTH_CX, MOUTH_OY, 11, MOUTH_COLOR);
    M5.Display.fillCircle(MOUTH_CX, MOUTH_OY, 6, BG_COLOR);
}
static void mouth_line() {  // flat (thinking)
    M5.Display.fillRoundRect(MOUTH_CX - 18, MOUTH_OY, 36, 5, 2, MOUTH_COLOR);
}
static void mouth_talk(int openPx) {  // animated speaking
    M5.Display.fillEllipse(MOUTH_CX, MOUTH_OY, 18, openPx, MOUTH_COLOR);
}
static void draw_think_dots(int phase) {  // phase 0..3 dots
    auto& lcd = M5.Display; int oy = 16;
    int y = 30 + oy, x0 = MOUTH_CX - 18;
    lcd.fillRect(x0 - 4, y - 4, 48, 12, BG_COLOR);
    for (int i = 0; i < 3; i++)
        lcd.fillCircle(x0 + i * 16, y, 3, (i < phase) ? 0xFFFFFF : 0x333355);
}

// === Per-state full-face render ===
static void face_idle(bool blink) {
    clear_face();
    if (blink) draw_blink_eyes(); else draw_eyes(4, 2, false, false);
    draw_cheeks();
    mouth_smile();
}
static void face_listen() {
    clear_face();
    draw_eyes(2, -5, true, false);  // wide, looking up: attentive
    draw_cheeks();
    mouth_o();
}
static void face_think() {
    clear_face();
    draw_eyes(9, -7, false, false);  // glance up-right: pensive
    draw_cheeks();
    mouth_line();
}
static void face_speak() {
    clear_face();
    draw_eyes(4, 2, false, false);
    draw_cheeks();
    mouth_talk(3);
}
static void face_sleep() {
    auto& lcd = M5.Display;
    clear_face();
    draw_eyes(0, 0, false, true);  // closed
    lcd.setTextColor(0x9999BB);
    lcd.setFont(&fonts::Font0);
    lcd.setTextSize(2); lcd.setCursor(248, 70); lcd.print("z");
    lcd.setTextSize(3); lcd.setCursor(262, 44); lcd.print("Z");
}

// === Face state manager: draw on change + per-state animation ===
enum class FaceMode { Sleep, Idle, Listen, Think, Speak };
static FaceMode face_mode = FaceMode::Idle;
static FaceMode last_face = (FaceMode)-1;

// Drive the procedural pet character from system state.
static void update_face(uint32_t now) {
    (void)now;
    static bool wasAsleep = false;
    if (asleep != wasAsleep) {
        wasAsleep = asleep;
        M5.Display.setBrightness(asleep ? SLEEP_BRIGHTNESS : AWAKE_BRIGHTNESS);
    }
    pet::setAsleep(asleep);
    pet::setListening(audio_state == AudioState::Listening);
    pet::setThinking(audio_state == AudioState::Waiting);
    pet::setSpeaking(audio_state == AudioState::Speaking && M5.Speaker.isPlaying());
    // emotion is transient: hold it while speaking, then revert to neutral 4s later so the face blinks again
    if (current_emotion != "neutral") {
        if (audio_state == AudioState::Speaking && M5.Speaker.isPlaying()) emotion_at = millis();
        else if (millis() - emotion_at > 4000) current_emotion = "neutral";
    }
    pet::setMoodByName(current_emotion.c_str());
}

// === Hardware (LED + servo) driven by state + emotion ===
static void update_hardware(uint32_t now) {
    // ---- RGB LED: write only when color changes ----
    static int lr = -1, lg = -1, lb = -1;
    int r = 0, g = 0, b = 0;
    if (asleep) {
        r = 0; g = 0; b = 0;
    } else if (man_led) {
        r = man_r; g = man_g; b = man_b;             // manual override
    } else switch (audio_state) {
        case AudioState::Listening: r = 0;  g = 10; b = 90; break;   // attentive blue
        case AudioState::Waiting:   r = 90; g = 45; b = 0;  break;   // thinking amber
        case AudioState::Speaking:
            if      (current_emotion == "happy")     { r = 95;  g = 70; b = 15; }
            else if (current_emotion == "sad")       { r = 0;   g = 25; b = 90; }
            else if (current_emotion == "angry")     { r = 110; g = 0;  b = 0;  }
            else if (current_emotion == "surprised") { r = 0;   g = 85; b = 95; }
            else if (current_emotion == "love")      { r = 110; g = 25; b = 55; }
            else if (current_emotion == "confused")  { r = 60;  g = 30; b = 80; }
            else                                     { r = 70;  g = 55; b = 40; }
            break;
        default: r = 8; g = 8; b = 12; break;                        // idle dim
    }
    if (r != lr || g != lg || b != lb) { lr = r; lg = g; lb = b; hw::led(r, g, b); }

    // ---- Servo pose ----
    static uint32_t last_srv = 0;
    if (now - last_srv < 100) return;
    last_srv = now;

    // one-shot gesture takes priority
    if (gesture != 0) {
        float e = (now - gesture_start) / 1000.0f;
        if (now - gesture_start >= GESTURE_MS) { gesture = 0; }
        else if (gesture == 1) {  // nod (up-only): level->up->level x2
            int pitch = (int)(16 * fabsf(sinf(e * 2.0f * PI * 1.6f)));
            hw::look(man_yaw_set ? man_yaw_deg : 0, pitch, 100); return;
        } else {                  // shake: yaw left-right x2
            int yaw = (int)(20 * sinf(e * 2.0f * PI * 1.8f));
            hw::look(yaw, man_pitch_set ? man_pitch_deg : 0, 100); return;
        }
    }
    // manual held pose
    if (man_yaw_set || man_pitch_set) {
        hw::look(man_yaw_set ? man_yaw_deg : 0, man_pitch_set ? man_pitch_deg : 0, 300);
        return;
    }

    float t = now / 1000.0f;
    int yaw = 0, pitch = 0;
    switch (audio_state) {
        case AudioState::Listening:
            yaw = 0; pitch = 10; break;                              // attentive, look up
        case AudioState::Waiting: {
            // think: hold a slightly-up pose, only occasionally glance
            static uint32_t next_fidget = 0; static int gaze = 0;
            if (now > next_fidget) { next_fidget = now + 3000 + (esp_random() % 4000); gaze = (int)(esp_random() % 31) - 15; }
            yaw = gaze; pitch = 8; break;
        }
        case AudioState::Speaking:
            yaw = (int)(6 * sinf(t * 1.1f)); pitch = 4 + (int)(4 * sinf(t * 3.5f)); break;  // gentle up-bob
        default: yaw = 0; pitch = 8; break;                          // idle: head raised a little
    }
    hw::look(yaw, pitch, 300);
}

static void sendHello() {
    JsonDocument doc;
    doc["type"] = "hello";
    doc["device_id"] = configStore.getDeviceId();
    doc["firmware_version"] = "0.4.0";
    JsonObject audio = doc["audio"].to<JsonObject>();
    audio["input_format"] = "pcm";
    audio["input_sample_rate"] = 16000;
    audio["input_channels"] = 1;
    audio["input_frame_duration_ms"] = 20;
    audio["output_format"] = "pcm";
    audio["output_sample_rate"] = 16000;
    audio["output_channels"] = 1;
    JsonArray caps = doc["capabilities"].to<JsonArray>();
    caps.add("servo");
    caps.add("led");
    caps.add("face");
    caps.add("battery");
    String json;
    serializeJson(doc, json);
    webSocket.sendTXT(json);
}

// === Report device status (battery, volume) so the agent can answer queries ===
static void send_status() {
    if (ws_state < 1) return;
    JsonDocument doc;
    doc["type"] = "status";
    doc["battery"] = M5.Power.getBatteryLevel();   // 0..100
    // isCharging() returns an enum (is_charging_t), not bool — cast so JSON
    // emits true/false (server expects a JSON boolean, not integer 0/1).
    doc["charging"] = ((int)M5.Power.isCharging() != 0);
    doc["volume"] = cur_volume_pct;
    // report current appearance selection so the server can persist it
    {
        JsonDocument ap;
        deserializeJson(ap, pet::appearanceJson());
        ap.remove("bg");              // bg is server-owned; don't let the device clobber it
        doc["appearance"] = ap;
    }
    String json;
    serializeJson(doc, json);
    webSocket.sendTXT(json);
}

// === Audio output: receive streamed PCM frames from server ===
static void handleAudioOutput(uint8_t* payload, size_t length) {
    if (length < 4) return;
    uint8_t type = payload[0];
    uint8_t flags = payload[1];
    if (type != 0x02) return;  // AUDIO_OUTPUT
    if (flags & 0x01) {        // START of a reply
        play_bytes = 0;
        play_pos = 0;
        server_done = false;
        speaker_pending = true;  // loop switches mic→speaker
    }
    size_t n = length - 4;
    if (play_bytes + n > PLAY_MAX_BYTES) n = PLAY_MAX_BYTES - play_bytes;
    if (n > 0) {
        memcpy(play_buf + play_bytes, payload + 4, n);
        play_bytes += n;
    }
}

// === Send recorded PCM up to the server as audio_input frames ===
static void send_audio_frame(const uint8_t* data, size_t len, uint8_t flags) {
    size_t fs = 4 + len;
    uint8_t* f = (uint8_t*)malloc(fs);
    if (!f) return;
    f[0] = 0x01;  // AUDIO_INPUT
    f[1] = flags;
    f[2] = (len >> 8) & 0xff;
    f[3] = len & 0xff;
    memcpy(f + 4, data, len);
    webSocket.sendBIN(f, fs);
    free(f);
}

static void send_recording() {
    size_t total = rec_samples * 2;  // bytes
    const uint8_t* p = (const uint8_t*)rec_buf;
    size_t off = 0;
    bool first = true;
    const size_t CHUNK = 8000;
    while (off < total) {
        size_t end = off + CHUNK;
        if (end > total) end = total;
        bool last = end >= total;
        uint8_t flags = 0;
        if (first) { flags |= 0x01; first = false; }
        if (last) flags |= 0x02;
        send_audio_frame(p + off, end - off, flags);
        off = end;
    }
    Serial.printf("[Audio] sent %u bytes PCM\n", (unsigned)total);
}

static void go_to_sleep() {
    audio_state = AudioState::Idle;
    asleep = true;
    man_led = false;          // release manual overrides on sleep
    man_yaw_set = false;
    man_pitch_set = false;
    gesture = 0;
    M5.Display.setBrightness(SLEEP_BRIGHTNESS);
}

static void start_listening(uint32_t now_ms, bool followup) {
    M5.Speaker.end();
    M5.Mic.begin();
    rec_samples = 0;
    chunk_start = 0;
    chunk_pending = false;
    speech_detected = false;
    voiced_run = 0;
    voiced_total = 0;
    noise_floor = 0;  // adaptive VAD: re-seed ambient each turn
    followup_listen = followup;
    listen_start_ms = now_ms;
    baseline_until = now_ms + 600;   // first 600ms = measure ambient noise (anti-noise VAD)
    last_voice_ms = now_ms;
    last_touch_ms = now_ms;  // hold-to-talk: touching at start
    last_activity_ms = now_ms;  // any new listening turn resets idle/sleep timer
    audio_state = AudioState::Listening;
}

static void finish_listening(bool send, uint32_t now_ms) {
    while (M5.Mic.isRecording()) { delay(1); }
    M5.Mic.end();
    if (send && rec_samples >= (size_t)(SAMPLE_RATE / 2)) {  // >=0.5s recorded -> send
        // Switch to THINK and render it *before* uploading. send_recording()
        // blocks while it pushes audio to the (remote) server, so if we flipped
        // state after, the UI would sit on LIS during the whole upload.
        audio_state = AudioState::Waiting;
        waiting_since = now_ms;
        update_face(now_ms);
        draw_status_bar();
        send_recording();
    } else {
        M5.Speaker.begin();
        M5.Speaker.setVolume(255);
        go_to_sleep();
    }
}

static void handleServerJson(uint8_t* payload, size_t length) {
    JsonDocument doc;
    if (deserializeJson(doc, payload, length)) return;
    const char* type = doc["type"];
    if (!type) return;
    if (strcmp(type, "hello") == 0) {
        ws_state = 2;
    } else if (strcmp(type, "response") == 0) {
        const char* emotion = doc["emotion"];
        if (emotion) { current_emotion = emotion; emotion_at = millis(); face_dirty = true; }
    } else if (strcmp(type, "speak_done") == 0) {
        server_done = true;  // no more audio for this reply
    } else if (strcmp(type, "sync") == 0) {
        // server-authoritative state: growth + gender + appearance
        pet::setStats(doc["level"] | 1, doc["xp"] | 0, doc["xp_need"] | 100,
                      doc["bond"] | 0, doc["energy"] | 0);
        const char* gender = doc["gender"] | "girl";
        String ap;
        if (!doc["appearance"].isNull()) serializeJson(doc["appearance"], ap);
        pet::applySync(gender, ap.c_str());
    } else if (strcmp(type, "control") == 0) {
        const char* action = doc["action"];
        if (!action) return;
        if (strcmp(action, "volume") == 0) {
            int v = doc["value"] | cur_volume_pct;
            if (v < 0) v = 0; if (v > 100) v = 100;
            cur_volume_pct = v;
            M5.Speaker.setVolume(v * 255 / 100);
        } else if (strcmp(action, "led") == 0) {
            const char* c = doc["color"];
            if (c && strcmp(c, "auto") == 0) { man_led = false; }
            else if (doc["r"].is<int>()) {
                man_led = true; man_r = doc["r"]; man_g = doc["g"]; man_b = doc["b"];
            } else if (c) {
                man_led = true;
                struct { const char* n; uint8_t r, g, b; } tbl[] = {
                    {"red",110,0,0}, {"green",0,110,0}, {"blue",0,20,120}, {"white",90,90,90},
                    {"yellow",100,80,0}, {"orange",110,40,0}, {"purple",70,0,110},
                    {"pink",120,30,60}, {"cyan",0,90,100}, {"off",0,0,0},
                };
                man_r = man_g = man_b = 0;
                for (auto& e : tbl) if (strcmp(c, e.n) == 0) { man_r = e.r; man_g = e.g; man_b = e.b; break; }
            }
        } else if (strcmp(action, "turn") == 0) {
            const char* d = doc["dir"];
            if (!d) return;
            if (strcmp(d, "auto") == 0) { man_yaw_set = false; man_pitch_set = false; }
            else if (strcmp(d, "left") == 0)   { man_yaw_set = true; man_yaw_deg = -20; }
            else if (strcmp(d, "right") == 0)  { man_yaw_set = true; man_yaw_deg = 20; }
            else if (strcmp(d, "up") == 0)     { man_pitch_set = true; man_pitch_deg = 25; }
            else if (strcmp(d, "down") == 0)   { man_pitch_set = true; man_pitch_deg = 0; }  // base limit: level
            else if (strcmp(d, "center") == 0) { man_yaw_set = true; man_yaw_deg = 0; man_pitch_set = true; man_pitch_deg = 0; }
            else if (strcmp(d, "nod") == 0)    { gesture = 1; gesture_start = millis(); }
            else if (strcmp(d, "shake") == 0)  { gesture = 2; gesture_start = millis(); }
        } else if (strcmp(action, "wear") == 0) {
            const char* n = doc["name"];
            if (n) { pet::wear(n); send_status(); }   // persist new outfit immediately                       // change outfit/hair variant
        } else if (strcmp(action, "blush") == 0) {
            pet::setBlush((int)(doc["value"] | 0) != 0);
            send_status();   // report new appearance immediately (server persists it)
        } else if (strcmp(action, "glasses") == 0) {
            pet::setAccessory((int)(doc["value"] | 0) != 0);
            send_status();
        } else if (strcmp(action, "char") == 0) {
            const char* n = doc["name"];
            if (n) pet::setCharacter(n);
        } else if (strcmp(action, "bg") == 0) {
            const char* n = doc["name"];
            pet::setBg(n ? n : "");
        }
    } else if (strcmp(type, "tool_call") == 0) {
        const char* call_id = doc["call_id"];
        JsonDocument res;
        res["type"] = "tool_result";
        res["call_id"] = call_id ? call_id : "";
        res["status"] = "ok";
        res["result"].to<JsonObject>();
        String out;
        serializeJson(res, out);
        webSocket.sendTXT(out);
    }
}

void webSocketEvent(WStype_t type, uint8_t* payload, size_t length) {
    switch (type) {
        case WStype_CONNECTED:
            ws_state = 1;
            sendHello();
            send_status();
            break;
        case WStype_TEXT:
            handleServerJson(payload, length);
            break;
        case WStype_BIN:
            handleAudioOutput(payload, length);
            break;
        case WStype_DISCONNECTED:
            ws_state = 0;
            server_done = false;
            speaker_pending = false;
            play_bytes = 0;
            play_pos = 0;
            audio_state = AudioState::Idle;  // recover from any in-flight turn
            break;
        default:
            break;
    }
}

static void splash_screen() {
    auto& lcd = M5.Display;
    lcd.fillScreen(BG_COLOR);
    lcd.setTextDatum(textdatum_t::middle_center);
    lcd.setTextColor(0x9999BB);
    lcd.setFont(&fonts::Font4);
    lcd.setTextSize(1);
    lcd.drawString("hello", SCREEN_W / 2, SCREEN_H / 2 - 34);
    lcd.setTextColor(0xFFD24A);
    lcd.setFont(&fonts::Font4);
    lcd.setTextSize(2);
    lcd.drawString("kura", SCREEN_W / 2, SCREEN_H / 2 + 18);
    lcd.setTextDatum(textdatum_t::top_left);  // restore default for other drawing
    lcd.setTextSize(1);
}

void setup() {
    auto cfg = M5.config();
    M5.begin(cfg);
    Serial.begin(115200);
    M5.Display.setRotation(1);
    M5.Display.setBrightness(128);
    splash_screen();
    delay(1800);
    configStore.begin();
    // Apply runtime-tunable params from /config.json
    {
        VadConfig v = configStore.getVad();
        VAD_RISE_FACTOR = v.rise_factor;
        VAD_KEEP_FACTOR = v.keep_factor;
        VAD_MIN_MARGIN = v.min_margin;
        VAD_END_SILENCE_MS = v.end_silence_ms;
        NO_SPEECH_MS = v.no_speech_ms;
        VAD_MIN_RUN = v.min_run;
        AudioConfig a = configStore.getAudio();
        PREBUFFER_BYTES = (size_t)((uint64_t)SAMPLE_RATE * 2 * a.prebuffer_ms / 1000);
    }
    head_touch_init();
    hw::init();
    // ---- SD (TF) probe BEFORE the render task starts (shared SPI bus is free) ----
    {
        int sck = M5.getPin(m5::pin_name_t::sd_spi_sclk);
        int miso = M5.getPin(m5::pin_name_t::sd_spi_miso);
        int mosi = M5.getPin(m5::pin_name_t::sd_spi_mosi);
        int cs = M5.getPin(m5::pin_name_t::sd_spi_cs);
        char buf[280];
        SPI.begin(sck, miso, mosi, cs);
        bool ok = SD.begin(cs, SPI, 20000000);
        if (!ok) {
            snprintf(buf, sizeof buf, "SD mount FAIL pins sck=%d miso=%d mosi=%d cs=%d", sck, miso, mosi, cs);
        } else {
            uint8_t t = SD.cardType();
            uint64_t mb = SD.cardSize() / (1024ULL * 1024ULL);
            int n = 0; String names;
            File root = SD.open("/");
            if (root) {
                File f;
                while ((f = root.openNextFile())) {
                    if (n < 10) names += String(f.name()) + (f.isDirectory() ? "/ " : " ");
                    n++; f.close();
                }
                root.close();
            }
            snprintf(buf, sizeof buf, "SD ok type=%u size=%lluMB entries=%d [%s] (sck=%d cs=%d)",
                     t, (unsigned long long)mb, n, names.c_str(), sck, cs);
        }
        Serial.println(buf);
    }
    pet::init(configStore.getPetCharacter().c_str());   // image-based pet renderer (loads /pet/<char>/ from SD)
    pet::setServer(configStore.getServerHost().c_str(), configStore.getServerPort());  // for fetching art over HTTP
    pet::statsBegin();  // load cached growth (offline display before first sync)

    // Allocate audio buffers in PSRAM
    rec_buf = (int16_t*)heap_caps_malloc(REC_MAX_SAMPLES * sizeof(int16_t), MALLOC_CAP_SPIRAM);
    play_buf = (uint8_t*)heap_caps_malloc(PLAY_MAX_BYTES, MALLOC_CAP_SPIRAM);
    // Speaker on by default; switch to Mic only while recording.
    M5.Speaker.begin();
    M5.Speaker.setVolume(255);

    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    // Robust association on WPA2/WPA3-mixed nets: scan all channels and pick the
    // strongest BSSID instead of latching onto the first match (FAST_SCAN default).
    WiFi.setScanMethod(WIFI_ALL_CHANNEL_SCAN);
    WiFi.setSortMethod(WIFI_CONNECT_AP_BY_SIGNAL);
    // Log association result + disconnect reason code so failures are diagnosable.
    WiFi.onEvent([](WiFiEvent_t e, WiFiEventInfo_t info) {
        if (e == ARDUINO_EVENT_WIFI_STA_CONNECTED) {
            Serial.println("[WiFi] STA_CONNECTED");
        } else if (e == ARDUINO_EVENT_WIFI_STA_GOT_IP) {
            Serial.printf("[WiFi] GOT_IP %s\n", WiFi.localIP().toString().c_str());
        } else if (e == ARDUINO_EVENT_WIFI_STA_DISCONNECTED) {
            Serial.printf("[WiFi] DISCONNECTED reason=%d\n",
                          info.wifi_sta_disconnected.reason);
        }
    });
    {
        auto wifis = configStore.getWifiList();
        if (!wifis.empty()) {
            Serial.printf("[WiFi] connecting to '%s'...\n", wifis[0].ssid.c_str());
            WiFi.begin(wifis[0].ssid.c_str(), wifis[0].password.c_str());
        } else {
            WiFi.begin("YOUR_WIFI_SSID", "YOUR_WIFI_PASSWORD");
        }
    }
    uint32_t t0 = millis();
    while (WiFi.status() != WL_CONNECTED && millis() - t0 < 15000) delay(100);
    Serial.printf("[WiFi] result: status=%d ip=%s\n",
                  WiFi.status(), WiFi.localIP().toString().c_str());
    draw_status_bar();

    String headers = String("Authorization: Bearer ") + configStore.getApiKey() +
                     "\r\nX-Device-Id: " + configStore.getDeviceId();
    webSocket.setExtraHeaders(headers.c_str());
    webSocket.onEvent(webSocketEvent);
    webSocket.setReconnectInterval(5000);
    // NOTE: no enableHeartbeat — the server processes a turn synchronously and
    // can't answer pings during long inference, so heartbeat would false-trip a
    // disconnect mid-THINK. On LAN the idle TCP stays alive fine.
    Serial.printf("[WS] dialing %s:%d%s\n",
                  configStore.getServerHost().c_str(),
                  configStore.getServerPort(),
                  configStore.getServerPath().c_str());
    webSocket.begin(configStore.getServerHost().c_str(),
                    configStore.getServerPort(),
                    configStore.getServerPath().c_str());
    last_activity_ms = millis();
}

void loop() {
    webSocket.loop();
    M5.update();

    uint32_t now = millis();

    // === Screen touch: toggle the status screen. Edge-detected; independent of
    // the head sensor (hold-to-talk). Entering suspends the pet render task and
    // draws the status page; leaving resumes the pet view. ===
    {
        auto td = M5.Touch.getDetail();
        static bool prev_screen = false;
        bool pressed = td.isPressed();
        bool screen_tap = pressed && !prev_screen;
        prev_screen = pressed;
        if (screen_tap) {
            if (ui_screen == UiScreen::Pet) {
                ui_screen = UiScreen::Status;
                char sbuf[640]; build_status_text(sbuf, sizeof sbuf);
                pet::setStatusText(sbuf);
                pet::showStatus(true);
                status_last_draw = now;
            } else {
                ui_screen = UiScreen::Pet;
                pet::showStatus(false);
            }
        }
    }

    // === Tap to talk: tap head once to start listening, tap again to send. ===
    // (Energy VAD is unreliable in noisy rooms, so the turn is ended by a tap.
    // screen touch intentionally not used as a trigger.)
    bool talk = head_touched();
    static bool prev_talk = false;
    bool tap = talk && !prev_talk;        // rising edge = a fresh tap
    prev_talk = talk;
    if (tap && ws_state >= 1) {            // head pat -> report event (server adds bond/xp)
        webSocket.sendTXT("{\"type\":\"event\",\"kind\":\"head_pat\"}");
    }
    bool started_now = false;
    static bool stop_req = false;

    // Wake from sleep on tap → start listening
    if (asleep && tap) {
        asleep = false;
        M5.Display.setBrightness(AWAKE_BRIGHTNESS);
        if (ws_state >= 1) { start_listening(now, false); started_now = true; }
        last_activity_ms = now;
        draw_status_bar();
    }
    if (talk || audio_state != AudioState::Idle) {
        last_activity_ms = now;
    }
    // Awake + idle + tap → start listening
    if (!asleep && audio_state == AudioState::Idle && tap && ws_state >= 1) {
        start_listening(now, false);
        started_now = true;
        draw_status_bar();
    }
    if (started_now) {
        stop_req = false;
        // jump back to the pet view when a turn starts so the user sees the face
        if (ui_screen == UiScreen::Status) { ui_screen = UiScreen::Pet; pet::showStatus(false); }
    }

    // Listening: keep recording until a second tap (or max length). A latch
    // catches the transient tap edge even while the mic chunk is in flight.
    if (audio_state == AudioState::Listening) {
        if (talk) last_touch_ms = now;
        if (tap && !started_now && (now - listen_start_ms > MIN_HOLD_MS)) stop_req = true;

        if (!M5.Mic.isRecording()) {
            if (chunk_pending) {
                int64_t sum = 0;
                size_t n = rec_samples - chunk_start;
                for (size_t i = chunk_start; i < rec_samples; i++) {
                    int v = rec_buf[i];
                    sum += (v < 0) ? -v : v;
                }
                vad_level = n ? (uint32_t)(sum / (int64_t)n) : 0;
                chunk_pending = false;

                // --- noise-robust VAD: first calibrate the ambient floor over a
                // short baseline window (track the peak), then FREEZE it so the
                // thresholds clear venue noise — in loud rooms a drifting floor let
                // noise masquerade as speech and the turn never ended. ---
                if (now < baseline_until) {
                    if ((float)vad_level > noise_floor) noise_floor = (float)vad_level;
                } else {
                    if (noise_floor <= 0) noise_floor = (float)vad_level;
                    float rise = noise_floor * VAD_RISE_FACTOR + VAD_MIN_MARGIN;
                    float keep = noise_floor * VAD_KEEP_FACTOR + (VAD_MIN_MARGIN / 2);
                    if ((float)vad_level > rise) {
                        voiced_run++;
                        last_voice_ms = now;
                        if (voiced_run >= VAD_MIN_RUN) speech_detected = true;
                    } else {
                        voiced_run = 0;
                        if (speech_detected && (float)vad_level > keep) last_voice_ms = now;
                    }
                }
            }
            bool too_long = rec_samples + 1600 > REC_MAX_SAMPLES;
            // submit after speech falls back to the floor for VAD_END_SILENCE_MS
            bool ended = speech_detected && (now - last_voice_ms > VAD_END_SILENCE_MS);
            bool give_up = !speech_detected && (now - listen_start_ms > NO_SPEECH_MS);
            if (stop_req || ended || too_long) {
                stop_req = false;
                finish_listening(true, now);
                draw_status_bar();
            } else if (give_up) {
                finish_listening(false, now);  // woke but nobody spoke -> sleep
                draw_status_bar();
            } else {
                chunk_start = rec_samples;
                M5.Mic.record(rec_buf + rec_samples, 1600, SAMPLE_RATE);
                rec_samples += 1600;
                chunk_pending = true;
            }
        }
    }

    // Streaming playback: first audio frame switches mic→speaker
    if (speaker_pending) {
        speaker_pending = false;
        asleep = false;          // incoming audio (reply or proactive push) wakes the device
        last_activity_ms = now;  // AI answer arriving → reset idle/sleep timer
        M5.Mic.end();
        M5.Speaker.begin();
        M5.Speaker.setVolume(255);
        audio_state = AudioState::Speaking;
        draw_status_bar();
    }
    if (audio_state == AudioState::Speaking) {
        // Jitter buffer: don't start playback until a cushion is buffered (or the
        // whole reply arrived). Once started (play_pos>0) keep feeding as data comes.
        bool ready = (play_pos > 0) || server_done || (play_bytes >= PREBUFFER_BYTES);
        // feed accumulated PCM to the speaker queue; feed eagerly (whatever is
        // available) so the speaker queue stays full — the prebuffered play_buf
        // is the cushion, and withholding data here just causes underrun/stutter.
        while (ready && play_pos < play_bytes) {
            size_t avail = play_bytes - play_pos;
            size_t piece = (avail < PLAY_PIECE ? avail : PLAY_PIECE) & ~((size_t)1);
            if (piece == 0) break;
            if (!M5.Speaker.playRaw((const int16_t*)(play_buf + play_pos), piece / 2,
                                    SAMPLE_RATE, false, 1, 0)) {
                break;  // speaker queue full; retry next loop
            }
            play_pos += piece;
            last_activity_ms = now;
        }
        // reply fully played → go idle; the sleep countdown starts from NOW
        if (server_done && play_pos >= play_bytes && !M5.Speaker.isPlaying()) {
            audio_state = AudioState::Idle;   // stay awake, idle
            last_activity_ms = now;           // idle timer starts only now
            draw_status_bar();
        }
    }
    // Safety net only: recover if the turn never completes (no response at all).
    // Inference can take minutes, so keep this generous; a real disconnect resets
    // immediately via WStype_DISCONNECTED.
    if (audio_state == AudioState::Waiting && now - waiting_since > 300000) {
        go_to_sleep();
        draw_status_bar();
    }
    // Ensure mic is released back to speaker whenever idle
    if (audio_state == AudioState::Idle && M5.Mic.isEnabled()) {
        M5.Mic.end();
        M5.Speaker.begin();
        M5.Speaker.setVolume(255);
    }

    // Enter sleep after inactivity
    if (!asleep && audio_state == AudioState::Idle && now - last_activity_ms > SLEEP_AFTER_MS) {
        asleep = true;
        M5.Display.setBrightness(SLEEP_BRIGHTNESS);
    }

    // Face: state-driven expressions + per-state animation
    // Face/status: drive the pet character in Pet mode; in Status mode refresh
    // the status text ~1Hz (the pet render task draws it — single LCD owner).
    if (ui_screen == UiScreen::Status) {
        if (now - status_last_draw > 1000) {
            status_last_draw = now;
            char sbuf[640]; build_status_text(sbuf, sizeof sbuf);
            pet::setStatusText(sbuf);
        }
    } else {
        update_face(now);
    }
    update_hardware(now);
    if (pet::consumeLevelUp()) pet::react(pet::React::Hop);  // celebrate level-up (server-driven)

    static uint32_t last_status = 0;
    if (now - last_status > 1000) {
        last_status = now;
        draw_status_bar();
    }
    static uint32_t last_status_report = 0;
    if (now - last_status_report > 30000) {
        last_status_report = now;
        send_status();
    }
}
