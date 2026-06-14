#include "wifi_manager.h"
#include "../config/config_store.h"

void WifiManager::begin() {
    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    wifi_list_ = configStore.getWifiList();
    state_ = WifiState::Disconnected;
    Serial.printf("[WiFi] Manager initialized, %d networks configured\n", wifi_list_.size());
}

void WifiManager::update() {
    switch (state_) {
        case WifiState::Disconnected: {
            uint32_t now = millis();
            if (now - last_attempt_ms_ > RETRY_INTERVAL_MS) {
                tryNextNetwork();
            }
            break;
        }
        case WifiState::Connecting: {
            if (WiFi.status() == WL_CONNECTED) {
                state_ = WifiState::Connected;
                Serial.printf("[WiFi] Connected to %s, IP: %s\n",
                    WiFi.SSID().c_str(), WiFi.localIP().toString().c_str());
            } else if (millis() - connect_start_ms_ > CONNECT_TIMEOUT_MS) {
                last_error_ = "Timeout(status=" + String(WiFi.status()) + ")";
                Serial.printf("[WiFi] %s\n", last_error_.c_str());
                WiFi.disconnect();
                current_credential_index_++;  // round-robin to next network
                state_ = WifiState::Disconnected;
                last_attempt_ms_ = millis();
            }
            break;
        }
        case WifiState::Connected: {
            if (WiFi.status() != WL_CONNECTED) {
                Serial.println("[WiFi] Connection lost, reconnecting...");
                state_ = WifiState::Disconnected;
                last_attempt_ms_ = millis() - RETRY_INTERVAL_MS;
            }
            break;
        }
    }
}

bool WifiManager::isConnected() {
    return state_ == WifiState::Connected;
}

WifiState WifiManager::getState() {
    return state_;
}

String WifiManager::getIP() {
    return WiFi.localIP().toString();
}

String WifiManager::getConnectedSSID() {
    return WiFi.SSID();
}

String WifiManager::getConnectingSSID() {
    if (wifi_list_.empty()) return "";
    int idx = current_credential_index_ % (int)wifi_list_.size();
    return wifi_list_[idx].ssid;
}

String WifiManager::getLastError() {
    return last_error_;
}

int WifiManager::getRSSI() {
    return WiFi.RSSI();
}

void WifiManager::tryNextNetwork() {
    if (wifi_list_.empty()) {
        last_error_ = "No WiFi configured";
        Serial.println("[WiFi] No networks configured");
        last_attempt_ms_ = millis();
        return;
    }
    current_credential_index_ %= (int)wifi_list_.size();
    const WifiEntry& e = wifi_list_[current_credential_index_];
    Serial.printf("[WiFi] Calling WiFi.begin(%s)...\n", e.ssid.c_str());
    last_error_ = "begin() called: " + e.ssid;
    WiFi.begin(e.ssid.c_str(), e.password.c_str());
    state_ = WifiState::Connecting;
    connect_start_ms_ = millis();
    last_attempt_ms_ = millis();
    Serial.printf("[WiFi] WiFi.status() = %d\n", WiFi.status());
}
