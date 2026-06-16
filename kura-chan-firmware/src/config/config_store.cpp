#include "config_store.h"
#include <LittleFS.h>
#include <ArduinoJson.h>

ConfigStore configStore;

static const char* CFG_PATH = "/config.json";

void ConfigStore::loadDefaults_() {
    // Placeholders only — NO real credentials baked into firmware. Real values
    // come from /config.json (uploadfs, or future web UI). A fresh device with
    // no config.json boots with these and simply won't connect until configured.
    wifi_ = { {"YOUR_WIFI_SSID", "YOUR_WIFI_PASSWORD"} };
    srvHost_ = "YOUR_SERVER_HOST";
    srvPort_ = 8866;
    srvPath_ = "/ws/device";
    apiKey_ = "dev_key_001";
    deviceId_ = "KURA_CHAN_001";
    vad_ = {2.0f, 1.4f, 150, 700, 6000, 3};
    audio_ = {2000};
}

void ConfigStore::begin() {
    loadDefaults_();
    if (!LittleFS.begin(true)) {  // format on first use if needed
        Serial.println("[Config] LittleFS mount failed; using built-in defaults");
        return;
    }
    if (LittleFS.exists(CFG_PATH)) {
        if (load_()) Serial.println("[Config] loaded /config.json");
        else Serial.println("[Config] /config.json invalid; using defaults");
    } else {
        Serial.println("[Config] no /config.json; writing defaults");
        save_();
    }
    dump();
}

bool ConfigStore::load_() {
    File f = LittleFS.open(CFG_PATH, "r");
    if (!f) return false;
    JsonDocument doc;
    DeserializationError err = deserializeJson(doc, f);
    f.close();
    if (err) return false;

    srvHost_ = doc["server"]["host"] | srvHost_;
    srvPort_ = doc["server"]["port"] | srvPort_;
    srvPath_ = doc["server"]["path"] | srvPath_;
    apiKey_ = doc["auth"]["api_key"] | apiKey_;
    deviceId_ = doc["auth"]["device_id"] | deviceId_;

    if (doc["wifi"].is<JsonArray>()) {
        std::vector<WifiEntry> list;
        for (JsonObject w : doc["wifi"].as<JsonArray>()) {
            String ssid = w["ssid"] | "";
            if (ssid.length() > 0) list.push_back({ssid, String(w["pass"] | "")});
        }
        if (!list.empty()) wifi_ = list;  // keep defaults if the array is empty
    }

    vad_.rise_factor = doc["vad"]["rise_factor"] | vad_.rise_factor;
    vad_.keep_factor = doc["vad"]["keep_factor"] | vad_.keep_factor;
    vad_.min_margin = doc["vad"]["min_margin"] | vad_.min_margin;
    vad_.end_silence_ms = doc["vad"]["end_silence_ms"] | vad_.end_silence_ms;
    vad_.no_speech_ms = doc["vad"]["no_speech_ms"] | vad_.no_speech_ms;
    vad_.min_run = doc["vad"]["min_run"] | vad_.min_run;

    audio_.prebuffer_ms = doc["audio"]["prebuffer_ms"] | audio_.prebuffer_ms;
    return true;
}

bool ConfigStore::save_() {
    JsonDocument doc;
    JsonArray wa = doc["wifi"].to<JsonArray>();
    for (auto& w : wifi_) {
        JsonObject o = wa.add<JsonObject>();
        o["ssid"] = w.ssid;
        o["pass"] = w.password;
    }
    doc["server"]["host"] = srvHost_;
    doc["server"]["port"] = srvPort_;
    doc["server"]["path"] = srvPath_;
    doc["auth"]["api_key"] = apiKey_;
    doc["auth"]["device_id"] = deviceId_;
    doc["vad"]["rise_factor"] = vad_.rise_factor;
    doc["vad"]["keep_factor"] = vad_.keep_factor;
    doc["vad"]["min_margin"] = vad_.min_margin;
    doc["vad"]["end_silence_ms"] = vad_.end_silence_ms;
    doc["vad"]["no_speech_ms"] = vad_.no_speech_ms;
    doc["vad"]["min_run"] = vad_.min_run;
    doc["audio"]["prebuffer_ms"] = audio_.prebuffer_ms;

    File f = LittleFS.open(CFG_PATH, "w");
    if (!f) {
        Serial.println("[Config] save failed: cannot open for write");
        return false;
    }
    serializeJsonPretty(doc, f);
    f.close();
    return true;
}

std::vector<WifiEntry> ConfigStore::getWifiList() { return wifi_; }

void ConfigStore::addWifi(const String& ssid, const String& password) {
    if (wifi_.size() >= 5) wifi_.erase(wifi_.begin());
    wifi_.push_back({ssid, password});
    save_();
    Serial.printf("[Config] WiFi added: %s (%d total)\n", ssid.c_str(), (int)wifi_.size());
}

void ConfigStore::clearWifi() { wifi_.clear(); save_(); Serial.println("[Config] WiFi cleared"); }

String ConfigStore::getServerHost() { return srvHost_; }
uint16_t ConfigStore::getServerPort() { return srvPort_; }
String ConfigStore::getServerPath() { return srvPath_; }
String ConfigStore::getApiKey() { return apiKey_; }
String ConfigStore::getDeviceId() { return deviceId_; }
VadConfig ConfigStore::getVad() { return vad_; }
AudioConfig ConfigStore::getAudio() { return audio_; }

void ConfigStore::setServerHost(const String& host) { srvHost_ = host; save_(); Serial.printf("[Config] Server host: %s\n", host.c_str()); }
void ConfigStore::setServerPort(uint16_t port) { srvPort_ = port; save_(); Serial.printf("[Config] Server port: %d\n", port); }
void ConfigStore::setApiKey(const String& key) { apiKey_ = key; save_(); }
void ConfigStore::setDeviceId(const String& id) { deviceId_ = id; save_(); }

void ConfigStore::dump() {
    Serial.println("=== Kura-chan Config (LittleFS /config.json) ===");
    Serial.printf("  Server: %s:%d%s\n", srvHost_.c_str(), srvPort_, srvPath_.c_str());
    Serial.printf("  Device: %s\n", deviceId_.c_str());
    Serial.printf("  WiFi (%d):\n", (int)wifi_.size());
    for (auto& w : wifi_) Serial.printf("    - %s\n", w.ssid.c_str());
    Serial.printf("  VAD: rise=%.2f keep=%.2f margin=%lu end=%lums nospeech=%lums run=%d\n",
                  vad_.rise_factor, vad_.keep_factor, (unsigned long)vad_.min_margin,
                  (unsigned long)vad_.end_silence_ms, (unsigned long)vad_.no_speech_ms, vad_.min_run);
    Serial.printf("  Audio: prebuffer=%lums\n", (unsigned long)audio_.prebuffer_ms);
    Serial.println("================================================");
}
