#pragma once

#include <Arduino.h>

// Wi-Fi credentials (supports multiple networks, connects to first available)
struct WifiCredential {
    const char* ssid;
    const char* password;
};

static const WifiCredential WIFI_CREDENTIALS[] = {
    {"YOUR_WIFI_SSID_1", "YOUR_WIFI_PASSWORD_1"},
    {"YOUR_WIFI_SSID_2", "YOUR_WIFI_PASSWORD_2"},
};
static const int WIFI_CREDENTIAL_COUNT = sizeof(WIFI_CREDENTIALS) / sizeof(WIFI_CREDENTIALS[0]);

// Server configuration
static const char* SERVER_HOST = "192.168.1.100";  // Your Mac's LAN IP
static const uint16_t SERVER_PORT = 8080;
static const char* SERVER_PATH = "/ws/device";

// Device authentication
static const char* API_KEY = "dev_key_001";
static const char* DEVICE_ID = "KURA_CHAN_001";
static const char* FIRMWARE_VERSION = "0.2.0";
