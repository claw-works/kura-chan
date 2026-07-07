#include "camera.h"

#include <Arduino.h>
#include <esp_heap_caps.h>
#include "esp_camera.h"
#include "img_converters.h"
#include "mbedtls/base64.h"

// === M5Stack CoreS3 (GC0308) camera ===
//
// SCCB (camera control I2C) is wired to the *internal* I2C bus on GPIO12/11 —
// the same bus M5Unified already drives via Arduino Wire1 on I2C_NUM_1 (shared
// with the AXP2101 PMIC / AW9523 / touch). So we reuse that existing bus
// (pin_sccb_sda = -1, sccb_i2c_port = 1) instead of installing a second, port-1
// I2C driver, which would clash with M5's. The GC0308 responds at its own
// address on that shared bus.
//
// The tool_call handler runs inside webSocket.loop() on the Arduino core, which
// is the same context that drives In_I2C (LED/touch), so camera init's SCCB
// traffic never races another I2C user. The display render task (core 0) only
// touches SPI/SD, not I2C.

static bool camera_init() {
    camera_config_t config = {};
    config.pin_pwdn      = -1;
    config.pin_reset     = -1;
    config.pin_xclk      = 2;
    config.pin_sccb_sda  = -1;   // reuse M5 In_I2C (Wire1 @ I2C_NUM_1)
    config.pin_sccb_scl  = -1;
    config.sccb_i2c_port = 1;
    config.pin_d7 = 47; config.pin_d6 = 48; config.pin_d5 = 16; config.pin_d4 = 15;
    config.pin_d3 = 42; config.pin_d2 = 41; config.pin_d1 = 40; config.pin_d0 = 39;
    config.pin_vsync = 46;
    config.pin_href  = 38;
    config.pin_pclk  = 45;
    config.xclk_freq_hz = 20000000;
    config.ledc_timer   = LEDC_TIMER_0;
    config.ledc_channel = LEDC_CHANNEL_0;
    config.pixel_format = PIXFORMAT_RGB565;   // GC0308 has no hardware JPEG
    config.frame_size   = FRAMESIZE_VGA;      // 640x480
    config.jpeg_quality = 12;                 // unused for RGB565
    config.fb_count     = 1;
    config.fb_location  = CAMERA_FB_IN_PSRAM;
    config.grab_mode    = CAMERA_GRAB_WHEN_EMPTY;

    esp_err_t err = esp_camera_init(&config);
    if (err != ESP_OK) {
        Serial.printf("[CAM] init failed: 0x%x\n", err);
        return false;
    }
    return true;
}

namespace cam {

char* capture_jpeg_base64(size_t* out_len) {
    if (out_len) *out_len = 0;

    // Init on demand; deinit after each shot so the ~600KB VGA framebuffer
    // isn't held in PSRAM between photos. Reusing the shared SCCB bus means
    // deinit does NOT touch M5's Wire1 driver.
    if (!camera_init()) return nullptr;

    // GC0308's first frame after power-up is often stale/dark (AE not settled);
    // drop one and grab the next.
    camera_fb_t* fb = esp_camera_fb_get();
    if (fb) { esp_camera_fb_return(fb); fb = esp_camera_fb_get(); }
    if (!fb) {
        Serial.println("[CAM] fb_get failed");
        esp_camera_deinit();
        return nullptr;
    }

    // Software-encode RGB565 -> JPEG (jpg allocated in internal RAM by the lib).
    uint8_t* jpg = nullptr;
    size_t jpg_len = 0;
    bool ok = frame2jpg(fb, 80, &jpg, &jpg_len);
    esp_camera_fb_return(fb);
    esp_camera_deinit();

    if (!ok || !jpg || jpg_len == 0) {
        Serial.println("[CAM] jpeg encode failed");
        if (jpg) free(jpg);
        return nullptr;
    }

    // Base64-encode into a PSRAM buffer.
    size_t b64_cap = 4 * ((jpg_len + 2) / 3) + 1;
    char* b64 = (char*)heap_caps_malloc(b64_cap, MALLOC_CAP_SPIRAM);
    if (!b64) {
        Serial.println("[CAM] base64 alloc failed");
        free(jpg);
        return nullptr;
    }
    size_t olen = 0;
    int rc = mbedtls_base64_encode((unsigned char*)b64, b64_cap, &olen, jpg, jpg_len);
    free(jpg);
    if (rc != 0) {
        Serial.printf("[CAM] base64 failed: -0x%x\n", -rc);
        free(b64);
        return nullptr;
    }
    b64[olen] = 0;
    Serial.printf("[CAM] photo ok: jpeg=%uB base64=%uB\n",
                  (unsigned)jpg_len, (unsigned)olen);
    if (out_len) *out_len = olen;
    return b64;
}

} // namespace cam
