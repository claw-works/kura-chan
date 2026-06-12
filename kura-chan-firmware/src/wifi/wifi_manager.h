#pragma once

#include <Arduino.h>
#include <WiFi.h>

enum class WifiState {
    Disconnected,
    Connecting,
    Connected,
};

class WifiManager {
public:
    void begin();
    void update();
    bool isConnected();
    WifiState getState();
    String getIP();
    String getConnectedSSID();
    int getRSSI();

private:
    WifiState state_ = WifiState::Disconnected;
    uint32_t last_attempt_ms_ = 0;
    int current_credential_index_ = 0;
    uint32_t connect_start_ms_ = 0;
    static constexpr uint32_t CONNECT_TIMEOUT_MS = 10000;
    static constexpr uint32_t RETRY_INTERVAL_MS = 5000;

    void tryNextNetwork();
};
