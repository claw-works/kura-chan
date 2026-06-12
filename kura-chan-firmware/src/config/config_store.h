#pragma once

#include <Arduino.h>
#include <Preferences.h>
#include <vector>

struct WifiEntry {
    String ssid;
    String password;
};

class ConfigStore {
public:
    void begin();

    // Wi-Fi (supports up to 5 networks)
    std::vector<WifiEntry> getWifiList();
    void addWifi(const String& ssid, const String& password);
    void clearWifi();

    // Server
    String getServerHost();
    uint16_t getServerPort();
    String getServerPath();
    String getApiKey();
    String getDeviceId();

    void setServerHost(const String& host);
    void setServerPort(uint16_t port);
    void setApiKey(const String& key);
    void setDeviceId(const String& id);

    // Debug: print all config
    void dump();

private:
    Preferences prefs_;
};

extern ConfigStore configStore;
