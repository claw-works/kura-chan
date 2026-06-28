# Kura-chan Architecture

## Overview

Kura-chan is an AI-powered desktop companion robot built on M5Stack Stack-chan (K151) hardware.
It uses a three-tier architecture: device firmware, relay server, and cloud AI runtime.

## System Architecture

```
┌──────────────────────────────┐
│  Kura-chan Device (ESP32-S3) │  C++ (Arduino + M5Stack BSP)
│                              │
│  Local:                      │
│  • Wake word detection       │
│  • Audio capture (Opus enc)  │
│  • Audio playback (Opus dec) │
│  • Tool execution            │
│    - servo_move(x, y)        │
│    - led_set(color, effect)  │
│    - face_set(emotion)       │
│    - camera_capture()        │
│    - ir_send(code)           │
│    - sensor_read(type)       │
│    - nfc_scan()              │
│                              │
└─────────────┬────────────────┘
              │ WebSocket (Opus audio + JSON control)
              ▼
┌──────────────────────────────┐
│  Kura-chan Server (Rust)     │  axum + tokio + aws-sdk-bedrockagentcore
│                              │
│  • Device authentication     │
│  • WebSocket session mgmt    │
│  • STT (speech → text)       │
│  • TTS (text → speech)       │
│  • Harness invocation        │
│  • Tool call routing         │
│    - inline → device         │
│    - gateway → Harness       │
│  • Audio codec bridging      │
│                              │
└─────────────┬────────────────┘
              │ aws-sdk streaming (invoke_harness)
              ▼
┌──────────────────────────────┐
│  AgentCore Harness (AWS)     │  Managed
│                              │
│  • Agent loop (Claude)       │
│  • Memory                    │
│    - Short-term (session)    │
│    - Long-term (cross-sess)  │
│  • Gateway MCP tools         │
│    - Weather / Calendar      │
│    - Smart home (HA)         │
│    - Custom integrations     │
│                              │
└──────────────────────────────┘
```

## Design Decisions

### Why a relay server (not direct device → Harness)?

1. **Auth complexity** — AWS SigV4 signing is heavy for ESP32; server holds credentials
2. **STT/TTS** — Audio processing needs more compute than ESP32 can provide
3. **Flexibility** — Swap STT/TTS providers without reflashing firmware
4. **Security** — Device uses simple API key; server manages AWS IAM

### Why WebSocket (not HTTP)?

1. **Bidirectional streaming** — Audio flows both ways simultaneously
2. **Low latency** — No connection setup per utterance
3. **Multiplexing** — Audio frames and control messages share one connection

### Why Opus codec?

1. **Low bitrate** — ~16kbps for speech, fits ESP32 Wi-Fi bandwidth
2. **Low latency** — Designed for real-time communication
3. **ESP32 support** — Hardware-friendly decode complexity
4. **Proven** — Used by xiaozhi-esp32 in production

## Data Flow: Voice Conversation

```
1. Wake:    ESP32 detects "Hi Kura" via local ESP-SR
2. Record:  ESP32 captures audio → Opus encode → WS binary frames → Server
3. STT:     Server accumulates audio → transcribes to text
4. Think:   Server calls invoke_harness(text + session_id)
5. Respond: Harness returns:
            a) Text reply → Server TTS → Opus audio → WS → Device plays
            b) Tool call → Server routes to device → Device executes → reports back
            c) Both → Execute tool first, then voice reply
6. Express: Server includes emotion tag → Device updates face animation
```

## Hardware Reference (M5Stack K151)

| Component | Spec |
|-----------|------|
| MCU | ESP32-S3, dual-core 240MHz, 16MB Flash, 8MB PSRAM |
| Display | 2.0" IPS 320x240, capacitive touch (FT6336U) |
| Camera | GC0308, 640x480 |
| Audio In | Dual mic + ES7210 codec |
| Audio Out | 1W speaker + AW88298 I2S amp |
| Motion | X-axis 360° continuous + Y-axis 90° (SCS0009 servos w/ feedback) |
| LEDs | 12x WS2812C RGB |
| Comms | Wi-Fi 2.4GHz, BLE 5, ESP-NOW, NFC (ST25R3916), IR TX/RX |
| Sensors | BMI270 (accel+gyro), BMM150 (mag), LTR-553ALS (proximity/light) |
| Power | 550mAh battery, AXP2101 PMU, BM8563 RTC |
| Expansion | 3x Grove (A/B/C), microSD, LEGO holes |
