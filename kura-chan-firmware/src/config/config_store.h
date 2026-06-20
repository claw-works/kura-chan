#pragma once

#include <Arduino.h>
#include <vector>

struct WifiEntry {
    String ssid;
    String password;
};

struct VadConfig {
    float rise_factor;      // speech start: energy > floor * rise_factor (+ margin)
    float keep_factor;      // still talking: energy > floor * keep_factor
    uint32_t min_margin;    // absolute energy margin above floor
    uint32_t end_silence_ms;// low this long after speech -> submit
    uint32_t no_speech_ms;  // give up if nobody speaks after wake
    int min_run;            // consecutive loud chunks to confirm speech
};

struct AudioConfig {
    uint32_t prebuffer_ms;  // jitter buffer before TTS playback starts
};

// Config is stored as /config.json on LittleFS (decoupled from firmware).
// Edit data/config.json + `pio run -t uploadfs`, or (future) a web UI on the
// device hotspot writes the same file. Missing/invalid file falls back to
// baked-in defaults (and writes them out), so the device never bricks.
class ConfigStore {
public:
    void begin();

    std::vector<WifiEntry> getWifiList();
    void addWifi(const String& ssid, const String& password);
    void clearWifi();

    String getServerHost();
    uint16_t getServerPort();
    String getServerPath();
    String getApiKey();
    String getDeviceId();

    VadConfig getVad();
    AudioConfig getAudio();
    String getPetCharacter();

    void setServerHost(const String& host);
    void setServerPort(uint16_t port);
    void setApiKey(const String& key);
    void setDeviceId(const String& id);

    void dump();

private:
    void loadDefaults_();
    bool load_();   // read /config.json into cache
    bool save_();   // write cache to /config.json

    std::vector<WifiEntry> wifi_;
    String srvHost_, srvPath_, apiKey_, deviceId_;
    uint16_t srvPort_ = 8866;
    VadConfig vad_{};
    AudioConfig audio_{};
    String petChar_;
};

extern ConfigStore configStore;
