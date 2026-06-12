#include "wifi_manager.h"
#include "../config/config.h"

void WifiManager::begin() {
    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    state_ = WifiState::Disconnected;
    Serial.println("[WiFi] Manager initialized");
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
                Serial.printf("[WiFi] Timeout connecting to %s\n",
                    WIFI_CREDENTIALS[current_credential_index_].ssid);
                WiFi.disconnect();
                state_ = WifiState::Disconnected;
                current_credential_index_ = (current_credential_index_ + 1) % WIFI_CREDENTIAL_COUNT;
                last_attempt_ms_ = millis();
            }
            break;
        }
        case WifiState::Connected: {
            if (WiFi.status() != WL_CONNECTED) {
                Serial.println("[WiFi] Connection lost, reconnecting...");
                state_ = WifiState::Disconnected;
                last_attempt_ms_ = millis() - RETRY_INTERVAL_MS; // Retry immediately
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
    const auto& cred = WIFI_CREDENTIALS[current_credential_index_];
    Serial.printf("[WiFi] Trying %s (%d/%d)...\n",
        cred.ssid, current_credential_index_ + 1, WIFI_CREDENTIAL_COUNT);

    WiFi.begin(cred.ssid, cred.password);
    state_ = WifiState::Connecting;
    connect_start_ms_ = millis();
    last_attempt_ms_ = millis();
}
