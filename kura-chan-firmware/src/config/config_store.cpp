#include "config_store.h"

ConfigStore configStore;

void ConfigStore::begin() {
    prefs_.begin("kura", false);

    // Write defaults on first boot (or after factory reset)
    if (!prefs_.isKey("v3")) {
        prefs_.clear();
        Serial.println("[Config] Initializing config...");
        prefs_.putBool("v3", true);
        prefs_.putString("srv_host", "54.187.154.83");
        prefs_.putUShort("srv_port", 8866);
        prefs_.putString("srv_path", "/ws/device");
        prefs_.putString("api_key", "dev_key_001");
        prefs_.putString("device_id", "KURA_CHAN_001");
        prefs_.putString("ws0", "松善");
        prefs_.putString("wp0", "66668888");
        prefs_.putUChar("wifi_cnt", 1);
    }

    // Config migration by revision. NVS keys must be <=15 chars, so we use a
    // single short "cfg_rev" counter instead of one boolean key per migration
    // (those long keys silently failed with KEY_TOO_LONG and re-ran every boot).
    // To re-point WiFi/server on already-provisioned devices: update the values
    // below and bump CFG_REV.
    constexpr uint8_t CFG_REV = 2;
    if (prefs_.getUChar("cfg_rev", 0) < CFG_REV) {
        prefs_.putString("ws0", "松善");
        prefs_.putString("wp0", "66668888");
        if (prefs_.getUChar("wifi_cnt", 0) < 1) {
            prefs_.putUChar("wifi_cnt", 1);
        }
        prefs_.putString("srv_host", "54.187.154.83");
        prefs_.putUShort("srv_port", 8866);
        prefs_.putUChar("cfg_rev", CFG_REV);
        Serial.println("[Config] migrated rev2: WiFi=松善 srv=54.187.154.83:8866");
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
