# Kura-chan Server

AI desktop companion relay server — bridges ESP32 device to AWS AgentCore Harness.

## Quick Start

```bash
cargo run
```

Server starts on `http://0.0.0.0:8080`.

## Test with wscat

```bash
# Install wscat
bun add -g wscat

# Connect with auth
wscat -c ws://127.0.0.1:8080/ws/device \
  -H "Authorization: Bearer dev_key_001" \
  -H "X-Device-Id: AA:BB:CC:DD:EE:FF"

# Send hello
{"type":"hello","device_id":"AA:BB:CC:DD:EE:FF","firmware_version":"0.1.0","audio":{"input_format":"opus","input_sample_rate":16000,"input_channels":1,"input_frame_duration_ms":20,"output_format":"opus","output_sample_rate":16000,"output_channels":1},"capabilities":["servo","led","camera"]}
```

Expected response:
```json
{"type":"hello","session_id":"ses_...","audio":{"output_format":"opus","output_sample_rate":16000,"output_channels":1,"output_frame_duration_ms":20},"server_version":"0.1.0"}
```

## Health Check

```bash
curl http://127.0.0.1:8080/health
# ok
```

## Integration Tests

Tests require the server to be running:

```bash
cargo run &
cargo test -- --ignored
```

## Configuration

Edit `config/default.toml` or use env vars with `KURA_` prefix:

```bash
KURA_SERVER_PORT=9090 cargo run
```

## Architecture

See `../docs/` for design documents.

## Current Status (V1)

- [x] WebSocket server with device authentication
- [x] Protocol: JSON control messages + binary audio frames
- [x] Session state machine (idle → listening → thinking → speaking)
- [x] Mock STT/TTS (placeholder for Volcengine real-time speech)
- [x] AgentCore Harness integration (invoke_inline_agent)
- [x] Tool call routing framework
- [ ] Real STT/TTS (Volcengine)
- [ ] Opus codec bridging
- [ ] Session timeout / reconnection
- [ ] Multiple device support
