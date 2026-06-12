# Kura-chan WebSocket Protocol

## Overview

Device and server communicate over a single WebSocket connection.
Two frame types: **JSON** (text frames) for control, **Binary** for audio.

## Connection

```
wss://server.example.com/ws/device
Headers:
  Authorization: Bearer <device_api_key>
  X-Device-Id: <mac_address>
  X-Firmware-Version: <version>
```

## Handshake

### Client Hello (device → server)

```json
{
  "type": "hello",
  "device_id": "AA:BB:CC:DD:EE:FF",
  "firmware_version": "0.1.0",
  "audio": {
    "input_format": "opus",
    "input_sample_rate": 16000,
    "input_channels": 1,
    "input_frame_duration_ms": 20,
    "output_format": "opus",
    "output_sample_rate": 16000,
    "output_channels": 1
  },
  "capabilities": ["servo", "led", "camera", "ir", "nfc", "imu"]
}
```

### Server Hello (server → device)

```json
{
  "type": "hello",
  "session_id": "ses_abc123",
  "audio": {
    "output_format": "opus",
    "output_sample_rate": 16000,
    "output_channels": 1,
    "output_frame_duration_ms": 20
  },
  "server_version": "0.1.0"
}
```

## Audio Frames (Binary)

### Frame Header (4 bytes)

```
┌─────────┬─────────┬──────────────────┐
│ type(1) │ flags(1)│ payload_len(2)   │
│  0x01   │         │  big-endian u16  │
└─────────┴─────────┴──────────────────┘
```

**Type**:
- `0x01` — Audio input (device → server)
- `0x02` — Audio output (server → device)

**Flags**:
- `0x01` — Start of utterance
- `0x02` — End of utterance
- `0x04` — Interrupt (cancel current playback)

**Payload**: Raw Opus packet (no additional framing)

## JSON Messages

### Listening State (server → device)

```json
{
  "type": "state",
  "state": "listening"
}
```

States: `idle`, `listening`, `thinking`, `speaking`

### STT Result (server → device)

```json
{
  "type": "stt",
  "text": "what time is it",
  "final": true
}
```

Partial results have `"final": false` for real-time display on LCD.

### Agent Response (server → device)

```json
{
  "type": "response",
  "text": "It's 3:30 PM!",
  "emotion": "happy",
  "audio_follows": true
}
```

`audio_follows: true` means binary audio frames will follow immediately.

### Tool Call (server → device)

```json
{
  "type": "tool_call",
  "call_id": "tc_001",
  "tool": "servo_move",
  "params": {
    "x": 180,
    "y": 45
  }
}
```

### Tool Result (device → server)

```json
{
  "type": "tool_result",
  "call_id": "tc_001",
  "status": "ok",
  "result": {}
}
```

### Error

```json
{
  "type": "error",
  "code": "auth_failed",
  "message": "Invalid API key"
}
```

## Device Tools

| Tool | Params | Description |
|------|--------|-------------|
| `servo_move` | `x: 0-360, y: 5-85` | Move head position |
| `servo_dance` | `sequence: [{x,y,ms}]` | Choreographed movement |
| `led_set` | `color: "#RRGGBB", effect: "solid\|breathe\|rainbow"` | Control RGB LEDs |
| `face_set` | `emotion: string` | Change face expression |
| `audio_play` | `url: string` | Play audio from URL |
| `camera_capture` | `{}` | Take photo, return base64 |
| `ir_send` | `protocol: string, code: string` | Send IR command |
| `sensor_read` | `type: "imu\|light\|proximity"` | Read sensor data |
| `nfc_scan` | `timeout_ms: u32` | Scan NFC tag |

## Emotions

Supported emotion values for `face_set` and response `emotion` field:

`neutral`, `happy`, `sad`, `angry`, `surprised`, `sleepy`, `thinking`, `love`, `dizzy`, `wink`

## Keepalive

Device sends ping every 30s. Server responds with pong.
If no ping received for 60s, server closes connection.

## Reconnection

Device should reconnect with exponential backoff:
1s → 2s → 4s → 8s → 16s → 30s (max)

Session resumes if reconnected within 5 minutes (same session_id).
