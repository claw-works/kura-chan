#include "ws_client.h"
#include <WiFiUdp.h>

WsClient* WsClient::instance_ = nullptr;

// TEMP DEBUG: report every raw WS library event via UDP
static void evt_beacon(WStype_t type, size_t length) {
    if (WiFi.status() != WL_CONNECTED) return;
    static WiFiUDP udp;
    udp.beginPacket(IPAddress(192,168,31,249), 8089);
    udp.printf("EVT type=%d len=%u", (int)type, (unsigned)length);
    udp.endPacket();
}

void WsClient::begin(const char* host, uint16_t port, const char* path,
                     const char* api_key, const char* device_id) {
    instance_ = this;

    // Build auth headers.
    // NOTE: the library appends a trailing CRLF itself (handshake += extraHeaders + "\r\n"),
    // so headers must be separated by "\r\n" but MUST NOT end with one — otherwise a blank
    // line terminates the HTTP headers early and the library's own User-Agent header leaks
    // into the WebSocket data stream (server sees byte 'U'=0x55 -> invalid opcode 5).
    String headers = String("Authorization: Bearer ") + api_key + "\r\n" +
                     "X-Device-Id: " + device_id;

    ws_.begin(host, port, path);
    ws_.setExtraHeaders(headers.c_str());
    ws_.onEvent(eventCallback);
    ws_.setReconnectInterval(5000); // Auto-reconnect every 5s

    setState(WsState::Connecting);
    Serial.printf("[WS] Connecting to %s:%d%s\n", host, port, path);
}

void WsClient::update() {
    ws_.loop();
}

void WsClient::disconnect() {
    ws_.disconnect();
    setState(WsState::Disconnected);
}

bool WsClient::isReady() {
    return state_ == WsState::Ready;
}

WsState WsClient::getState() {
    return state_;
}

void WsClient::sendHello(const char* device_id, const char* firmware_version) {
    JsonDocument doc;
    doc["type"] = "hello";
    doc["device_id"] = device_id;
    doc["firmware_version"] = firmware_version;

    JsonObject audio = doc["audio"].to<JsonObject>();
    audio["input_format"] = "opus";
    audio["input_sample_rate"] = 16000;
    audio["input_channels"] = 1;
    audio["input_frame_duration_ms"] = 20;
    audio["output_format"] = "opus";
    audio["output_sample_rate"] = 16000;
    audio["output_channels"] = 1;

    JsonArray caps = doc["capabilities"].to<JsonArray>();
    caps.add("servo");
    caps.add("led");
    caps.add("camera");
    caps.add("ir");
    caps.add("nfc");
    caps.add("imu");

    String json;
    serializeJson(doc, json);
    ws_.sendTXT(json);
    Serial.println("[WS] Hello sent");
}

void WsClient::sendToolResult(const char* call_id, const char* status, JsonDocument& result) {
    JsonDocument doc;
    doc["type"] = "tool_result";
    doc["call_id"] = call_id;
    doc["status"] = status;
    doc["result"] = result;

    String json;
    serializeJson(doc, json);
    ws_.sendTXT(json);
}

void WsClient::sendAudioFrame(uint8_t* data, size_t length, uint8_t flags) {
    // 4-byte header + payload
    size_t frame_size = 4 + length;
    uint8_t* frame = (uint8_t*)malloc(frame_size);
    if (!frame) return;

    frame[0] = 0x01; // AUDIO_INPUT
    frame[1] = flags;
    frame[2] = (length >> 8) & 0xFF;
    frame[3] = length & 0xFF;
    memcpy(frame + 4, data, length);

    ws_.sendBIN(frame, frame_size);
    free(frame);
}

void WsClient::setState(WsState new_state) {
    if (state_ != new_state) {
        state_ = new_state;
        if (state_cb_) state_cb_(new_state);
    }
}

void WsClient::handleEvent(WStype_t type, uint8_t* payload, size_t length) {
    evt_beacon(type, length);
    switch (type) {
        case WStype_CONNECTED:
            Serial.println("[WS] Connected");
            setState(WsState::Connected);
            reconnect_delay_ms_ = 1000; // Reset backoff
            break;

        case WStype_DISCONNECTED:
            Serial.println("[WS] Disconnected");
            last_disconnect_ms_ = millis();
            // Exponential backoff: 1s → 2s → 4s → 8s → 16s → 30s max
            reconnect_delay_ms_ = min(reconnect_delay_ms_ * 2, (uint32_t)30000);
            setState(WsState::Disconnected);
            break;

        case WStype_TEXT: {
            JsonDocument doc;
            DeserializationError err = deserializeJson(doc, payload, length);
            if (err) {
                Serial.printf("[WS] JSON parse error: %s\n", err.c_str());
                break;
            }

            const char* msg_type = doc["type"];
            if (msg_type && strcmp(msg_type, "hello") == 0) {
                const char* session_id = doc["session_id"];
                Serial.printf("[WS] Handshake complete, session: %s\n", session_id);
                setState(WsState::Ready);
            }

            if (json_cb_) json_cb_(doc);
            break;
        }

        case WStype_BIN:
            if (binary_cb_) binary_cb_(payload, length);
            break;

        case WStype_PING:
            Serial.println("[WS] Ping");
            break;

        case WStype_PONG:
            break;

        case WStype_ERROR:
            Serial.printf("[WS] Error: %s\n", payload ? (char*)payload : "unknown");
            break;

        default:
            break;
    }
}

void WsClient::eventCallback(WStype_t type, uint8_t* payload, size_t length) {
    if (instance_) {
        instance_->handleEvent(type, payload, length);
    }
}
