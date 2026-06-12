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
            if (wifi_list_.empty()) break;
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
                Serial.printf("[WiFi] Timeout connecting to %s\n",
                    wifi_list_[current_credential_index_].ssid.c_str());
                WiFi.disconnect();
                state_ = WifiState::Disconnected;
                current_credential_index_ = (current_credential_index_ + 1) % wifi_list_.size();
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

int WifiManager::getRSSI() {
    return WiFi.RSSI();
}

void WifiManager::tryNextNetwork() {
    const auto& entry = wifi_list_[current_credential_index_];
    Serial.printf("[WiFi] Trying %s (%d/%d)...\n",
        entry.ssid.c_str(), current_credential_index_ + 1, wifi_list_.size());

    WiFi.begin(entry.ssid.c_str(), entry.password.c_str());
    state_ = WifiState::Connecting;
    connect_start_ms_ = millis();
    last_attempt_ms_ = millis();
}
