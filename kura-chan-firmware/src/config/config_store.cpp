#include "config_store.h"

ConfigStore configStore;

void ConfigStore::begin() {
    prefs_.begin("kura", false);

    // First boot: write defaults if no config exists
    if (!prefs_.isKey("init")) {
        Serial.println("[Config] First boot, writing defaults...");
        prefs_.putBool("init", true);
        prefs_.putString("srv_host", "192.168.1.100");
        prefs_.putUShort("srv_port", 8080);
        prefs_.putString("srv_path", "/ws/device");
        prefs_.putString("api_key", "dev_key_001");
        prefs_.putString("device_id", "KURA_CHAN_001");
        prefs_.putUChar("wifi_cnt", 0);
    }
}

std::vector<WifiEntry> ConfigStore::getWifiList() {
    std::vector<WifiEntry> list;
    uint8_t count = prefs_.getUChar("wifi_cnt", 0);
    for (uint8_t i = 0; i < count && i < 5; i++) {
        String ssid = prefs_.getString(("ws" + String(i)).c_str(), "");
        String pass = prefs_.getString(("wp" + String(i)).c_str(), "");
        if (ssid.length() > 0) {
            list.push_back({ssid, pass});
        }
    }
    return list;
}

void ConfigStore::addWifi(const String& ssid, const String& password) {
    uint8_t count = prefs_.getUChar("wifi_cnt", 0);
    if (count >= 5) {
        Serial.println("[Config] Max 5 WiFi networks, removing oldest");
        // Shift all down
        for (uint8_t i = 0; i < 4; i++) {
            String s = prefs_.getString(("ws" + String(i + 1)).c_str(), "");
            String p = prefs_.getString(("wp" + String(i + 1)).c_str(), "");
            prefs_.putString(("ws" + String(i)).c_str(), s);
            prefs_.putString(("wp" + String(i)).c_str(), p);
        }
        count = 4;
    }
    prefs_.putString(("ws" + String(count)).c_str(), ssid);
    prefs_.putString(("wp" + String(count)).c_str(), password);
    prefs_.putUChar("wifi_cnt", count + 1);
    Serial.printf("[Config] WiFi added: %s (%d total)\n", ssid.c_str(), count + 1);
}

void ConfigStore::clearWifi() {
    prefs_.putUChar("wifi_cnt", 0);
    Serial.println("[Config] WiFi list cleared");
}

String ConfigStore::getServerHost() {
    return prefs_.getString("srv_host", "192.168.1.100");
}

uint16_t ConfigStore::getServerPort() {
    return prefs_.getUShort("srv_port", 8080);
}

String ConfigStore::getServerPath() {
    return prefs_.getString("srv_path", "/ws/device");
}

String ConfigStore::getApiKey() {
    return prefs_.getString("api_key", "dev_key_001");
}

String ConfigStore::getDeviceId() {
    return prefs_.getString("device_id", "KURA_CHAN_001");
}

void ConfigStore::setServerHost(const String& host) {
    prefs_.putString("srv_host", host);
    Serial.printf("[Config] Server host: %s\n", host.c_str());
}

void ConfigStore::setServerPort(uint16_t port) {
    prefs_.putUShort("srv_port", port);
    Serial.printf("[Config] Server port: %d\n", port);
}

void ConfigStore::setApiKey(const String& key) {
    prefs_.putString("api_key", key);
}

void ConfigStore::setDeviceId(const String& id) {
    prefs_.putString("device_id", id);
}

void ConfigStore::dump() {
    Serial.println("=== Kura-chan Config ===");
    Serial.printf("  Server: %s:%d%s\n",
        getServerHost().c_str(), getServerPort(), getServerPath().c_str());
    Serial.printf("  API Key: %s\n", getApiKey().c_str());
    Serial.printf("  Device ID: %s\n", getDeviceId().c_str());

    auto wifis = getWifiList();
    Serial.printf("  WiFi networks (%d):\n", wifis.size());
    for (auto& w : wifis) {
        Serial.printf("    - %s\n", w.ssid.c_str());
    }
    Serial.println("=======================");
}
