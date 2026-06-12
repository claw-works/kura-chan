# Kura-chan Server Design

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Language | Rust | Type safety, performance, aws-sdk-bedrockagentcore available |
| HTTP/WS | axum | Mature, tokio-native, good WebSocket support |
| Runtime | tokio | Async I/O, required by aws-sdk |
| AWS SDK | aws-sdk-bedrockagentcore | Direct invoke_harness + memory APIs |
| Audio | opus (crate) | Encode/decode for bridging |
| Config | figment or config | Multi-source config (file + env) |

## Project Structure

```
kura-chan-server/
├── Cargo.toml
├── src/
│   ├── main.rs                 # Entry point, server startup
│   ├── config.rs               # Configuration loading
│   ├── error.rs                # Error types
│   ├── auth/
│   │   └── mod.rs              # Device API key validation
│   ├── ws/
│   │   ├── mod.rs              # WebSocket upgrade handler
│   │   ├── session.rs          # Per-device session state
│   │   ├── protocol.rs         # Frame parsing/serialization
│   │   └── audio.rs            # Binary audio frame handling
│   ├── speech/
│   │   ├── mod.rs              # STT/TTS trait definitions
│   │   ├── stt.rs              # Speech-to-text implementation
│   │   └── tts.rs              # Text-to-speech implementation
│   ├── harness/
│   │   ├── mod.rs              # AgentCore Harness client
│   │   ├── invoke.rs           # invoke_harness streaming wrapper
│   │   └── tools.rs            # Tool definition registry
│   └── router.rs              # axum route definitions
├── config/
│   ├── default.toml            # Default configuration
│   └── production.toml         # Production overrides
└── tests/
    └── integration/
        └── ws_test.rs          # WebSocket integration tests
```

## Core Components

### 1. WebSocket Session Manager

Each connected device gets a `Session`:

```rust
struct Session {
    device_id: String,
    session_id: String,
    state: SessionState,           // idle, listening, thinking, speaking
    audio_buffer: Vec<u8>,         // Accumulated audio for STT
    harness_session_id: Option<String>,  // AgentCore session continuity
}
```

### 2. Speech Pipeline

```
Device audio → [Opus decode] → [VAD] → [STT engine] → text
text → [TTS engine] → [Opus encode] → Device playback
```

STT/TTS are behind traits for swappability:

```rust
#[async_trait]
trait SpeechToText: Send + Sync {
    async fn transcribe(&self, audio: &[u8], format: AudioFormat) -> Result<String>;
}

#[async_trait]
trait TextToSpeech: Send + Sync {
    async fn synthesize(&self, text: &str) -> Result<AudioStream>;
}
```

Candidate implementations:
- **STT**: AWS Transcribe Streaming, Whisper API, or Volcengine ASR
- **TTS**: AWS Polly, edge-tts, or Volcengine TTS

### 3. Harness Client

Wraps `aws-sdk-bedrockagentcore` with streaming response handling:

```rust
struct HarnessClient {
    client: aws_sdk_bedrockagentcore::Client,
    harness_arn: String,
}

impl HarnessClient {
    async fn invoke(&self, message: &str, session_id: &str) -> Result<HarnessResponseStream>;
}
```

Response stream yields:
- `TextChunk(String)` — partial text for TTS streaming
- `ToolCall { id, name, params }` — route to device
- `Done` — end of response

### 4. Tool Router

Decides where a tool call executes:

```rust
enum ToolTarget {
    Device,    // servo_move, led_set, face_set, etc.
    Gateway,   // Weather, calendar, smart home (handled by Harness)
}
```

Device tools are sent over WebSocket as JSON. Server waits for `tool_result` before
continuing the Harness conversation (tool results feed back into agent loop).

## Configuration

```toml
[server]
host = "0.0.0.0"
port = 8080

[auth]
# Device API keys (later: database-backed)
api_keys = ["key_device_001", "key_device_002"]

[aws]
region = "us-west-2"
harness_arn = "arn:aws:bedrock-agent-core:us-west-2:123456:harness/kura-chan"

[speech.stt]
provider = "whisper"  # or "aws_transcribe", "volcengine"

[speech.tts]
provider = "edge_tts"  # or "aws_polly", "volcengine"
voice = "zh-CN-XiaoxiaoNeural"

[session]
timeout_seconds = 300
max_audio_buffer_ms = 30000
```

## Deployment

Phase 1: Single binary, run locally or on a small VPS/EC2.
Phase 2: Containerized, deploy on ECS/Fargate if needed.

The server is stateless per-process (session state is in-memory per connection,
long-term state lives in AgentCore Memory). Horizontal scaling can use
sticky sessions by device_id if needed later.

## Security

- Device auth via API key in WebSocket upgrade header
- Server → AWS auth via IAM role / env credentials
- TLS required for WebSocket (wss://)
- Rate limiting per device connection
- Audio buffer size cap to prevent memory exhaustion
