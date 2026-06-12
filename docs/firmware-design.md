# Kura-chan Firmware Design

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Framework | Arduino + ESP-IDF | M5Stack BSP is Arduino-based |
| BSP | [StackChan-BSP](https://github.com/m5stack/StackChan-BSP) | Official hardware abstraction |
| Build | PlatformIO | Better dependency management than Arduino IDE |
| Audio codec | Opus (libopus) | Matches server protocol |
| Wake word | ESP-SR | Espressif's on-device speech recognition |
| Display | LVGL or M5GFX | Face animation rendering |

## Project Structure

```
kura-chan-firmware/
├── platformio.ini              # Build configuration
├── src/
│   ├── main.cpp                # Entry point, task creation
│   ├── config.h                # Pin definitions, constants
│   ├── wifi/
│   │   └── wifi_manager.cpp    # Wi-Fi connection + reconnection
│   ├── ws/
│   │   ├── ws_client.cpp       # WebSocket client
│   │   ├── protocol.cpp        # Frame parse/serialize
│   │   └── session.cpp         # Handshake + state machine
│   ├── audio/
│   │   ├── mic.cpp             # Microphone capture (I2S)
│   │   ├── speaker.cpp         # Speaker playback (I2S)
│   │   ├── opus_encoder.cpp    # Opus encoding
│   │   ├── opus_decoder.cpp    # Opus decoding
│   │   └── vad.cpp             # Voice activity detection
│   ├── wake/
│   │   └── wake_word.cpp       # ESP-SR wake word engine
│   ├── face/
│   │   ├── face_renderer.cpp   # Expression animation engine
│   │   └── emotions.h          # Emotion definitions
│   ├── motion/
│   │   ├── servo.cpp           # Servo control (UART serial bus)
│   │   └── dance.cpp           # Choreography sequences
│   ├── led/
│   │   └── led_strip.cpp       # WS2812C control
│   ├── tools/
│   │   ├── tool_executor.cpp   # Dispatch tool_call from server
│   │   ├── camera_tool.cpp     # Camera capture
│   │   ├── ir_tool.cpp         # IR send/receive
│   │   ├── nfc_tool.cpp        # NFC operations
│   │   └── sensor_tool.cpp     # IMU, light, proximity reads
│   └── power/
│       └── power_manager.cpp   # Battery monitoring, sleep
├── lib/
│   └── (local libraries)
└── data/
    └── (SPIFFS assets: sounds, config)
```

## FreeRTOS Task Architecture

ESP32-S3 has two cores. Tasks are pinned for performance:

| Task | Core | Priority | Stack | Purpose |
|------|------|----------|-------|---------|
| `audio_in` | Core 1 | High | 8KB | Mic capture → Opus encode → WS send |
| `audio_out` | Core 1 | High | 8KB | WS receive → Opus decode → Speaker |
| `ws_client` | Core 0 | Medium | 8KB | WebSocket read/write, reconnection |
| `face_render` | Core 0 | Medium | 4KB | LCD animation at 30fps |
| `tool_exec` | Core 0 | Low | 4KB | Execute tool_calls, report results |
| `wake_word` | Core 1 | High | 16KB | ESP-SR continuous listening |

## State Machine

```
                  ┌──────────┐
         wake     │          │  timeout (10s)
        ┌────────►│ LISTENING ├──────────┐
        │         │          │           │
        │         └────┬─────┘           │
        │              │ end_of_speech    │
   ┌────┴────┐         ▼                 │
   │         │    ┌──────────┐           │
   │  IDLE   │◄───┤ THINKING │           │
   │         │    └────┬─────┘           │
   └────▲────┘         │ response        │
        │              ▼                 │
        │         ┌──────────┐           │
        └─────────┤ SPEAKING │◄──────────┘
           done   │          │
                  └──────────┘
```

Interrupt: User can say wake word during SPEAKING → cancel playback → LISTENING.

## Audio Pipeline

### Input (Mic → Server)

```
I2S DMA (16kHz 16bit mono)
    → Ring buffer (200ms)
    → Wake word engine (always-on, low power)
    → [On wake] VAD-gated capture
    → Opus encode (20ms frames)
    → WebSocket binary send
```

### Output (Server → Speaker)

```
WebSocket binary receive
    → Jitter buffer (60ms)
    → Opus decode
    → Ring buffer
    → I2S DMA playback
```

## Face Animation

Expressions are rendered at 30fps on the 320x240 IPS display.

Each emotion defines:
- Eye shape (open, half, closed, star, heart, etc.)
- Eye animation (blink interval, look direction)
- Mouth shape (smile, neutral, open, speaking)
- Mouth animation (speaking sync with audio level)
- Optional particles (hearts, stars, zzz)

Transition between emotions uses 200ms ease-in-out interpolation.

## Servo Control

Servos use serial bus protocol (SCS0009) over UART:
- TX: GPIO6
- RX: GPIO7

Position feedback allows smooth acceleration/deceleration curves.
Movement commands specify target angle + duration for interpolated motion.

## Power Management

- Active conversation: ~200mA (Wi-Fi + audio + display + servo)
- Idle (display on, listening): ~120mA
- Light sleep (display dim, wake word only): ~40mA
- Deep sleep (button wake only): ~2mA

Auto-dim after 60s idle. Light sleep after 5min idle.
Wake from light sleep via wake word or touch.

## Configuration Storage

Device config stored in NVS (Non-Volatile Storage):

```
wifi_ssid, wifi_pass        — Wi-Fi credentials
server_url                  — WebSocket server URL
api_key                     — Device authentication key
device_name                 — Display name
volume                      — Speaker volume (0-100)
brightness                  — Display brightness (0-100)
```

Initial setup via BLE provisioning or captive portal AP mode.

## OTA Updates

Firmware updates via HTTPS from server:
1. Server notifies device of available update via JSON message
2. Device downloads firmware binary over HTTPS
3. Validates checksum
4. Writes to OTA partition
5. Reboots into new firmware
6. Rolls back if boot fails 3 times
