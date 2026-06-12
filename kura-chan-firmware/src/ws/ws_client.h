#pragma once

#include <Arduino.h>
#include <WebSocketsClient.h>
#include <ArduinoJson.h>

enum class WsState {
    Disconnected,
    Connecting,
    Connected,
    Ready,  // After hello handshake complete
};

class WsClient {
public:
    using JsonCallback = std::function<void(JsonDocument& doc)>;
    using BinaryCallback = std::function<void(uint8_t* payload, size_t length)>;
    using StateCallback = std::function<void(WsState state)>;

    void begin(const char* host, uint16_t port, const char* path,
               const char* api_key, const char* device_id);
    void update();
    void disconnect();
    bool isReady();
    WsState getState();

    void sendHello(const char* device_id, const char* firmware_version);
    void sendToolResult(const char* call_id, const char* status, JsonDocument& result);
    void sendAudioFrame(uint8_t* data, size_t length, uint8_t flags);

    void onJson(JsonCallback cb) { json_cb_ = cb; }
    void onBinary(BinaryCallback cb) { binary_cb_ = cb; }
    void onStateChange(StateCallback cb) { state_cb_ = cb; }

private:
    WebSocketsClient ws_;
    WsState state_ = WsState::Disconnected;
    JsonCallback json_cb_;
    BinaryCallback binary_cb_;
    StateCallback state_cb_;
    uint32_t reconnect_delay_ms_ = 1000;
    uint32_t last_disconnect_ms_ = 0;

    void setState(WsState new_state);
    void handleEvent(WStype_t type, uint8_t* payload, size_t length);
    static void eventCallback(WStype_t type, uint8_t* payload, size_t length);
    static WsClient* instance_;
};
