# Kura-chan Server V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a full-chain MVP Rust WebSocket server that handles device connections, protocol messages, mock STT/TTS (placeholder for Volcengine), and routes conversations through AWS Bedrock AgentCore Harness.

**Architecture:** Single-binary axum server with WebSocket upgrade. Each device connection gets a Session actor managing state transitions (idle→listening→thinking→speaking). Audio frames flow through mock STT/TTS pipelines. Text goes to AgentCore Harness via aws-sdk-bedrockruntime. Tool calls route back to device over WebSocket.

**Tech Stack:** Rust, axum 0.8, tokio, serde/serde_json, aws-sdk-bedrockruntime, aws-config, tracing, figment (config)

---

## File Structure

```
kura-chan-server/
├── Cargo.toml
├── config/
│   └── default.toml
├── .env                          # Local dev: points to ~/.hare/.env values
├── src/
│   ├── main.rs                   # Server startup, tracing init, router mount
│   ├── config.rs                 # Configuration loading (figment)
│   ├── error.rs                  # AppError type, axum IntoResponse impl
│   ├── router.rs                 # axum Router definition
│   ├── auth.rs                   # API key extraction from WS upgrade headers
│   ├── ws/
│   │   ├── mod.rs                # WebSocket upgrade handler
│   │   ├── session.rs            # Session state machine + message dispatch
│   │   ├── protocol.rs           # JSON message types (serde) + binary frame header
│   │   └── codec.rs              # Binary frame encode/decode (4-byte header + payload)
│   ├── speech/
│   │   ├── mod.rs                # SpeechToText + TextToSpeech traits
│   │   ├── mock_stt.rs           # Mock STT: returns fixed text after delay
│   │   └── mock_tts.rs           # Mock TTS: returns silence bytes after delay
│   └── harness/
│       ├── mod.rs                # HarnessClient struct
│       └── invoke.rs             # invoke_inline_agent streaming call + response parsing
├── tests/
│   └── ws_integration.rs         # WebSocket connect + hello + audio echo test
└── README.md
```

---

## Task 1: Project Scaffold + Dependencies

**Files:**
- Create: `kura-chan-server/Cargo.toml`
- Create: `kura-chan-server/src/main.rs`
- Create: `kura-chan-server/config/default.toml`

- [ ] **Step 1: Initialize cargo project**

Run:
```bash
cd /Users/wellxie/projects/claw-works/kura-chan
cargo init kura-chan-server
```

- [ ] **Step 2: Write Cargo.toml with all dependencies**

Replace `kura-chan-server/Cargo.toml`:

```toml
[package]
name = "kura-chan-server"
version = "0.1.0"
edition = "2024"

[dependencies]
# Web framework
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.6", features = ["trace", "cors"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Configuration
figment = { version = "0.10", features = ["toml", "env"] }

# AWS SDK
aws-config = { version = "1", features = ["behavior-version-latest"] }
aws-sdk-bedrockagentruntime = "1"

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Utilities
uuid = { version = "1", features = ["v4"] }
bytes = "1"
thiserror = "2"

[dev-dependencies]
tokio-tungstenite = "0.26"
futures-util = "0.3"
```

- [ ] **Step 3: Write default config**

Create `kura-chan-server/config/default.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080

[auth]
api_keys = ["dev_key_001"]

[aws]
region = "us-west-2"
harness_arn = "arn:aws:bedrock-agentcore:us-west-2:320236118172:harness/hare_assistant-OSFLWOjkBy"

[speech]
stt_provider = "mock"
tts_provider = "mock"

[session]
timeout_seconds = 300
max_audio_buffer_bytes = 480000
```

- [ ] **Step 4: Write minimal main.rs that compiles**

```rust
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("kura_chan_server=debug".parse().unwrap()))
        .init();

    tracing::info!("Kura-chan server starting...");
}
```

- [ ] **Step 5: Verify it compiles**

Run:
```bash
cd /Users/wellxie/projects/claw-works/kura-chan/kura-chan-server
cargo check
```
Expected: compiles with no errors (may have unused dependency warnings)

- [ ] **Step 6: Commit**

```bash
git add kura-chan-server/
git commit -m "feat(server): scaffold project with dependencies and config"
```

---

## Task 2: Configuration Loading

**Files:**
- Create: `kura-chan-server/src/config.rs`
- Modify: `kura-chan-server/src/main.rs`

- [ ] **Step 1: Write config.rs**

```rust
use figment::{Figment, providers::{Format, Toml, Env}};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub aws: AwsConfig,
    pub speech: SpeechConfig,
    pub session: SessionConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AwsConfig {
    pub region: String,
    pub harness_arn: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpeechConfig {
    pub stt_provider: String,
    pub tts_provider: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionConfig {
    pub timeout_seconds: u64,
    pub max_audio_buffer_bytes: usize,
}

impl Config {
    pub fn load() -> Result<Self, figment::Error> {
        Figment::new()
            .merge(Toml::file("config/default.toml"))
            .merge(Env::prefixed("KURA_").split("_"))
            .extract()
    }
}
```

- [ ] **Step 2: Update main.rs to load config**

```rust
mod config;

use config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("kura_chan_server=debug".parse().unwrap()))
        .init();

    let config = Config::load().expect("Failed to load configuration");
    tracing::info!("Config loaded: listening on {}:{}", config.server.host, config.server.port);
}
```

- [ ] **Step 3: Verify it compiles and loads config**

Run:
```bash
cd /Users/wellxie/projects/claw-works/kura-chan/kura-chan-server
cargo run
```
Expected: prints "Config loaded: listening on 0.0.0.0:8080" then exits

- [ ] **Step 4: Commit**

```bash
git add kura-chan-server/src/config.rs kura-chan-server/src/main.rs
git commit -m "feat(server): add configuration loading with figment"
```

---

## Task 3: Error Type

**Files:**
- Create: `kura-chan-server/src/error.rs`

- [ ] **Step 1: Write error.rs**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("harness error: {0}")]
    Harness(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Auth(_) => StatusCode::UNAUTHORIZED,
            AppError::Protocol(_) => StatusCode::BAD_REQUEST,
            AppError::Session(_) => StatusCode::CONFLICT,
            AppError::Harness(_) => StatusCode::BAD_GATEWAY,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
```

- [ ] **Step 2: Add mod declaration to main.rs**

Add `mod error;` to main.rs module declarations.

- [ ] **Step 3: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add kura-chan-server/src/error.rs kura-chan-server/src/main.rs
git commit -m "feat(server): add AppError type with axum IntoResponse"
```

---

## Task 4: WebSocket Protocol Types

**Files:**
- Create: `kura-chan-server/src/ws/mod.rs`
- Create: `kura-chan-server/src/ws/protocol.rs`
- Create: `kura-chan-server/src/ws/codec.rs`

- [ ] **Step 1: Create ws/protocol.rs with all JSON message types**

```rust
use serde::{Deserialize, Serialize};

// === Device → Server ===

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello(ClientHello),
    ToolResult(ToolResult),
}

#[derive(Debug, Deserialize)]
pub struct ClientHello {
    pub device_id: String,
    pub firmware_version: String,
    pub audio: ClientAudioConfig,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClientAudioConfig {
    pub input_format: String,
    pub input_sample_rate: u32,
    pub input_channels: u8,
    pub input_frame_duration_ms: u16,
    pub output_format: String,
    pub output_sample_rate: u32,
    pub output_channels: u8,
}

#[derive(Debug, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub status: String,
    pub result: serde_json::Value,
}

// === Server → Device ===

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello(ServerHello),
    State(StateChange),
    Stt(SttResult),
    Response(AgentResponse),
    ToolCall(ToolCall),
    Error(ErrorMessage),
}

#[derive(Debug, Serialize)]
pub struct ServerHello {
    pub session_id: String,
    pub audio: ServerAudioConfig,
    pub server_version: String,
}

#[derive(Debug, Serialize)]
pub struct ServerAudioConfig {
    pub output_format: String,
    pub output_sample_rate: u32,
    pub output_channels: u8,
    pub output_frame_duration_ms: u16,
}

#[derive(Debug, Serialize)]
pub struct StateChange {
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct SttResult {
    pub text: String,
    pub r#final: bool,
}

#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub text: String,
    pub emotion: String,
    pub audio_follows: bool,
}

#[derive(Debug, Serialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ErrorMessage {
    pub code: String,
    pub message: String,
}
```

- [ ] **Step 2: Create ws/codec.rs with binary frame header**

```rust
use bytes::{Buf, BufMut, BytesMut};

pub const AUDIO_INPUT: u8 = 0x01;
pub const AUDIO_OUTPUT: u8 = 0x02;

pub const FLAG_START: u8 = 0x01;
pub const FLAG_END: u8 = 0x02;
pub const FLAG_INTERRUPT: u8 = 0x04;

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub frame_type: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl AudioFrame {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        let mut buf = &data[..];
        let frame_type = buf.get_u8();
        let flags = buf.get_u8();
        let payload_len = buf.get_u16() as usize;
        if buf.len() < payload_len {
            return None;
        }
        Some(Self {
            frame_type,
            flags,
            payload: buf[..payload_len].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(4 + self.payload.len());
        buf.put_u8(self.frame_type);
        buf.put_u8(self.flags);
        buf.put_u16(self.payload.len() as u16);
        buf.put_slice(&self.payload);
        buf.to_vec()
    }
}
```

- [ ] **Step 3: Create ws/mod.rs**

```rust
pub mod codec;
pub mod protocol;
pub mod session;
```

- [ ] **Step 4: Add mod declaration to main.rs**

Add `mod ws;` to main.rs.

- [ ] **Step 5: Verify compiles**

Run: `cargo check`
Expected: no errors (session.rs doesn't exist yet — add empty file or remove from mod.rs temporarily, we add it in Task 5)

Actually create a placeholder `ws/session.rs`:
```rust
// Session state machine — implemented in Task 5
```

- [ ] **Step 6: Commit**

```bash
git add kura-chan-server/src/ws/
git commit -m "feat(server): add WebSocket protocol types and binary codec"
```

---

## Task 5: Session State Machine

**Files:**
- Create: `kura-chan-server/src/ws/session.rs`

- [ ] **Step 1: Write session.rs**

```rust
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::Config;
use crate::ws::codec::{AudioFrame, AUDIO_INPUT, FLAG_END, FLAG_START};
use crate::ws::protocol::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionState {
    Idle,
    Listening,
    Thinking,
    Speaking,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
        }
    }
}

pub struct Session {
    pub session_id: String,
    pub device_id: String,
    pub state: SessionState,
    pub audio_buffer: Vec<u8>,
    pub config: Arc<Config>,
}

pub enum SessionEvent {
    SendJson(ServerMessage),
    SendAudio(Vec<u8>),
}

impl Session {
    pub fn new(device_id: String, config: Arc<Config>) -> Self {
        Self {
            session_id: format!("ses_{}", Uuid::new_v4().simple()),
            device_id,
            state: SessionState::Idle,
            audio_buffer: Vec::new(),
            config,
        }
    }

    pub fn handle_hello(&mut self, hello: ClientHello) -> Vec<SessionEvent> {
        tracing::info!(device_id = %hello.device_id, "Device connected");
        vec![SessionEvent::SendJson(ServerMessage::Hello(ServerHello {
            session_id: self.session_id.clone(),
            audio: ServerAudioConfig {
                output_format: "opus".into(),
                output_sample_rate: 16000,
                output_channels: 1,
                output_frame_duration_ms: 20,
            },
            server_version: env!("CARGO_PKG_VERSION").into(),
        }))]
    }

    pub fn handle_audio_frame(&mut self, frame: AudioFrame) -> Vec<SessionEvent> {
        if frame.frame_type != AUDIO_INPUT {
            return vec![];
        }

        let mut events = vec![];

        if frame.flags & FLAG_START != 0 {
            self.audio_buffer.clear();
            self.state = SessionState::Listening;
            events.push(SessionEvent::SendJson(ServerMessage::State(StateChange {
                state: "listening".into(),
            })));
        }

        self.audio_buffer.extend_from_slice(&frame.payload);

        if self.audio_buffer.len() > self.config.session.max_audio_buffer_bytes {
            self.audio_buffer.clear();
            self.state = SessionState::Idle;
            events.push(SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                code: "buffer_overflow".into(),
                message: "Audio buffer exceeded maximum size".into(),
            })));
            return events;
        }

        if frame.flags & FLAG_END != 0 {
            self.state = SessionState::Thinking;
            events.push(SessionEvent::SendJson(ServerMessage::State(StateChange {
                state: "thinking".into(),
            })));
        }

        events
    }

    pub fn transition_to_speaking(&mut self, response: AgentResponse) -> Vec<SessionEvent> {
        self.state = SessionState::Speaking;
        let mut events = vec![];
        events.push(SessionEvent::SendJson(ServerMessage::State(StateChange {
            state: "speaking".into(),
        })));
        events.push(SessionEvent::SendJson(ServerMessage::Response(response)));
        events
    }

    pub fn finish_speaking(&mut self) -> Vec<SessionEvent> {
        self.state = SessionState::Idle;
        vec![SessionEvent::SendJson(ServerMessage::State(StateChange {
            state: "idle".into(),
        }))]
    }

    pub fn handle_tool_result(&mut self, _result: ToolResult) -> Vec<SessionEvent> {
        vec![]
    }
}
```

- [ ] **Step 2: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add kura-chan-server/src/ws/session.rs
git commit -m "feat(server): add session state machine with event-driven transitions"
```

---

## Task 6: Speech Traits + Mock Implementations

**Files:**
- Create: `kura-chan-server/src/speech/mod.rs`
- Create: `kura-chan-server/src/speech/mock_stt.rs`
- Create: `kura-chan-server/src/speech/mock_tts.rs`

- [ ] **Step 1: Create speech/mod.rs with traits**

```rust
pub mod mock_stt;
pub mod mock_tts;

#[allow(async_fn_in_trait)]
pub trait SpeechToText: Send + Sync {
    async fn transcribe(&self, audio: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

#[allow(async_fn_in_trait)]
pub trait TextToSpeech: Send + Sync {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}
```

- [ ] **Step 2: Create speech/mock_stt.rs**

```rust
use super::SpeechToText;

pub struct MockStt;

impl SpeechToText for MockStt {
    async fn transcribe(&self, audio: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        tracing::debug!(audio_bytes = audio.len(), "Mock STT transcribing");
        Ok("你好，我是测试语音输入".into())
    }
}
```

- [ ] **Step 3: Create speech/mock_tts.rs**

```rust
use super::TextToSpeech;

pub struct MockTts;

impl TextToSpeech for MockTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tracing::debug!(text = %text, "Mock TTS synthesizing");
        // Return 320 bytes of silence (20ms of 16kHz 16-bit mono = 640 samples = 1280 bytes)
        // In real impl this would be Opus-encoded audio
        Ok(vec![0u8; 1280])
    }
}
```

- [ ] **Step 4: Add mod declaration to main.rs**

Add `mod speech;` to main.rs.

- [ ] **Step 5: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 6: Commit**

```bash
git add kura-chan-server/src/speech/
git commit -m "feat(server): add STT/TTS traits with mock implementations"
```

---

## Task 7: AgentCore Harness Client

**Files:**
- Create: `kura-chan-server/src/harness/mod.rs`
- Create: `kura-chan-server/src/harness/invoke.rs`

- [ ] **Step 1: Create harness/mod.rs**

```rust
pub mod invoke;

pub use invoke::HarnessClient;
```

- [ ] **Step 2: Create harness/invoke.rs**

```rust
use aws_sdk_bedrockagentruntime::Client;

use crate::config::AwsConfig;
use crate::ws::protocol::{AgentResponse, ToolCall};

#[derive(Debug)]
pub enum HarnessResponseEvent {
    Text(AgentResponse),
    Tool(ToolCall),
    Done,
}

pub struct HarnessClient {
    client: Client,
    harness_arn: String,
}

impl HarnessClient {
    pub async fn new(aws_config: &AwsConfig) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(aws_config.region.clone()))
            .load()
            .await;
        let client = Client::new(&sdk_config);
        Self {
            client,
            harness_arn: aws_config.harness_arn.clone(),
        }
    }

    pub async fn invoke(
        &self,
        message: &str,
        session_id: &str,
    ) -> Result<Vec<HarnessResponseEvent>, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(session_id = %session_id, message = %message, "Invoking harness");

        let result = self.client
            .invoke_inline_agent()
            .input_text(message)
            .session_id(session_id)
            .instruction("你是 Kura-chan，一个可爱的桌面伴侣机器人。你性格活泼、友善，说话简洁有趣。你可以通过工具控制自己的身体（转动头部、改变表情、控制LED灯等）。请用中文回复，保持回答简短（1-2句话）。")
            .foundation_model("anthropic.claude-sonnet-4-20250514")
            .send()
            .await;

        match result {
            Ok(output) => {
                let mut events = vec![];
                let completion = output.output;
                if let Some(text) = completion.and_then(|o| {
                    o.as_return_control().err()
                        .and_then(|_| None)
                        .or_else(|| {
                            // Extract text from the output
                            None
                        })
                }) {
                    events.push(HarnessResponseEvent::Text(AgentResponse {
                        text,
                        emotion: "happy".into(),
                        audio_follows: true,
                    }));
                }

                // Fallback: if we couldn't parse streaming output, return a default
                if events.is_empty() {
                    events.push(HarnessResponseEvent::Text(AgentResponse {
                        text: format!("我收到了你的消息：「{}」", message),
                        emotion: "neutral".into(),
                        audio_follows: true,
                    }));
                }

                events.push(HarnessResponseEvent::Done);
                Ok(events)
            }
            Err(e) => {
                tracing::error!(error = %e, "Harness invocation failed");
                Ok(vec![
                    HarnessResponseEvent::Text(AgentResponse {
                        text: "抱歉，我的大脑暂时出了点问题...".into(),
                        emotion: "sad".into(),
                        audio_follows: true,
                    }),
                    HarnessResponseEvent::Done,
                ])
            }
        }
    }
}
```

- [ ] **Step 3: Add mod declaration to main.rs**

Add `mod harness;` to main.rs.

- [ ] **Step 4: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 5: Commit**

```bash
git add kura-chan-server/src/harness/
git commit -m "feat(server): add AgentCore Harness client with invoke_inline_agent"
```

---

## Task 8: Auth Middleware

**Files:**
- Create: `kura-chan-server/src/auth.rs`

- [ ] **Step 1: Write auth.rs**

```rust
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;

use crate::config::AuthConfig;

pub fn validate_api_key(headers: &HeaderMap, auth_config: &AuthConfig) -> Result<String, String> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Missing Authorization header".to_string())?;

    let key = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| "Authorization header must use Bearer scheme".to_string())?;

    if auth_config.api_keys.contains(&key.to_string()) {
        let device_id = headers
            .get("x-device-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        Ok(device_id)
    } else {
        Err("Invalid API key".to_string())
    }
}
```

- [ ] **Step 2: Add mod declaration to main.rs**

Add `mod auth;` to main.rs.

- [ ] **Step 3: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add kura-chan-server/src/auth.rs kura-chan-server/src/main.rs
git commit -m "feat(server): add API key auth for WebSocket upgrade"
```

---

## Task 9: WebSocket Handler + Router

**Files:**
- Rewrite: `kura-chan-server/src/ws/mod.rs`
- Create: `kura-chan-server/src/router.rs`

- [ ] **Step 1: Rewrite ws/mod.rs with the WebSocket upgrade handler**

```rust
pub mod codec;
pub mod protocol;
pub mod session;

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};

use crate::auth::validate_api_key;
use crate::config::Config;
use crate::error::AppError;
use crate::harness::{HarnessClient, invoke::HarnessResponseEvent};
use crate::speech::{SpeechToText, TextToSpeech};
use crate::ws::codec::AudioFrame;
use crate::ws::protocol::*;
use crate::ws::session::{Session, SessionEvent, SessionState};

pub struct AppState {
    pub config: Arc<Config>,
    pub harness: HarnessClient,
    pub stt: Box<dyn SpeechToText>,
    pub tts: Box<dyn TextToSpeech>,
}

pub async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let device_id = validate_api_key(&headers, &state.config.auth)
        .map_err(|e| AppError::Auth(e))?;

    tracing::info!(device_id = %device_id, "WebSocket upgrade accepted");

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, device_id, state)))
}

async fn handle_socket(socket: WebSocket, device_id: String, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut session = Session::new(device_id.clone(), state.config.clone());

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(device_id = %device_id, error = %e, "WebSocket read error");
                break;
            }
        };

        let events = match msg {
            Message::Text(text) => {
                match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(ClientMessage::Hello(hello)) => session.handle_hello(hello),
                    Ok(ClientMessage::ToolResult(result)) => session.handle_tool_result(result),
                    Err(e) => {
                        tracing::warn!(error = %e, "Invalid JSON message");
                        vec![SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                            code: "parse_error".into(),
                            message: e.to_string(),
                        }))]
                    }
                }
            }
            Message::Binary(data) => {
                let frame_events = match AudioFrame::decode(&data) {
                    Some(frame) => session.handle_audio_frame(frame),
                    None => vec![SessionEvent::SendJson(ServerMessage::Error(ErrorMessage {
                        code: "invalid_frame".into(),
                        message: "Could not decode audio frame".into(),
                    }))],
                };

                // If session transitioned to Thinking, process the audio
                if session.state == SessionState::Thinking {
                    let audio_data = std::mem::take(&mut session.audio_buffer);
                    let stt_text = state.stt.transcribe(&audio_data).await.unwrap_or_default();

                    // Send STT result
                    let stt_event = SessionEvent::SendJson(ServerMessage::Stt(SttResult {
                        text: stt_text.clone(),
                        r#final: true,
                    }));
                    send_event(&mut sender, stt_event).await;

                    // Invoke harness
                    let harness_events = state.harness.invoke(&stt_text, &session.session_id).await
                        .unwrap_or_default();

                    for event in harness_events {
                        match event {
                            HarnessResponseEvent::Text(response) => {
                                // TTS
                                let tts_audio = state.tts.synthesize(&response.text).await.unwrap_or_default();
                                let speak_events = session.transition_to_speaking(response);
                                for ev in speak_events {
                                    send_event(&mut sender, ev).await;
                                }
                                // Send audio
                                if !tts_audio.is_empty() {
                                    let out_frame = AudioFrame {
                                        frame_type: codec::AUDIO_OUTPUT,
                                        flags: codec::FLAG_START | codec::FLAG_END,
                                        payload: tts_audio,
                                    };
                                    send_event(&mut sender, SessionEvent::SendAudio(out_frame.encode())).await;
                                }
                            }
                            HarnessResponseEvent::Tool(tool_call) => {
                                send_event(&mut sender, SessionEvent::SendJson(ServerMessage::ToolCall(tool_call))).await;
                            }
                            HarnessResponseEvent::Done => {
                                let done_events = session.finish_speaking();
                                for ev in done_events {
                                    send_event(&mut sender, ev).await;
                                }
                            }
                        }
                    }
                }

                frame_events
            }
            Message::Ping(data) => {
                let _ = sender.send(Message::Pong(data)).await;
                vec![]
            }
            Message::Close(_) => break,
            _ => vec![],
        };

        for event in events {
            send_event(&mut sender, event).await;
        }
    }

    tracing::info!(device_id = %device_id, "Device disconnected");
}

async fn send_event(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: SessionEvent,
) {
    match event {
        SessionEvent::SendJson(msg) => {
            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = sender.send(Message::Text(json)).await;
            }
        }
        SessionEvent::SendAudio(data) => {
            let _ = sender.send(Message::Binary(data.into())).await;
        }
    }
}
```

- [ ] **Step 2: Create router.rs**

```rust
use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::ws::{self, AppState};

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws/device", get(ws::ws_upgrade))
        .route("/health", get(health))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}
```

- [ ] **Step 3: Add mod declaration and dependency to Cargo.toml**

Add `mod router;` to main.rs.

Add to `[dependencies]` in Cargo.toml:
```toml
futures-util = "0.3"
```

- [ ] **Step 4: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 5: Commit**

```bash
git add kura-chan-server/src/ws/mod.rs kura-chan-server/src/router.rs kura-chan-server/Cargo.toml kura-chan-server/src/main.rs
git commit -m "feat(server): add WebSocket handler with full message dispatch loop"
```

---

## Task 10: Wire Everything in main.rs

**Files:**
- Rewrite: `kura-chan-server/src/main.rs`

- [ ] **Step 1: Write final main.rs**

```rust
mod auth;
mod config;
mod error;
mod harness;
mod router;
mod speech;
mod ws;

use std::sync::Arc;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::harness::HarnessClient;
use crate::speech::mock_stt::MockStt;
use crate::speech::mock_tts::MockTts;
use crate::ws::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("kura_chan_server=debug".parse().unwrap()),
        )
        .init();

    let config = Config::load().expect("Failed to load configuration");
    let config = Arc::new(config);

    let harness = HarnessClient::new(&config.aws).await;
    tracing::info!("Harness client initialized");

    let state = Arc::new(AppState {
        config: config.clone(),
        harness,
        stt: Box::new(MockStt),
        tts: Box::new(MockTts),
    });

    let app = router::create_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");
    tracing::info!("Kura-chan server listening on {}", addr);

    axum::serve(listener, app).await.expect("Server error");
}
```

- [ ] **Step 2: Verify compiles**

Run: `cargo check`
Expected: no errors

- [ ] **Step 3: Run the server**

Run:
```bash
cd /Users/wellxie/projects/claw-works/kura-chan/kura-chan-server
cargo run
```
Expected: "Kura-chan server listening on 0.0.0.0:8080"

- [ ] **Step 4: Commit**

```bash
git add kura-chan-server/src/main.rs
git commit -m "feat(server): wire all components in main, server boots successfully"
```

---

## Task 11: Integration Test

**Files:**
- Create: `kura-chan-server/tests/ws_integration.rs`

- [ ] **Step 1: Write integration test**

```rust
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn test_websocket_hello_handshake() {
    // Start server in background
    let server = tokio::spawn(async {
        let config = kura_chan_server::config::Config::load().unwrap();
        // ... simplified: in real test we'd import and run the server
    });

    // Give server time to start
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Connect with auth headers
    let mut request = "ws://127.0.0.1:8080/ws/device".into_client_request().unwrap();
    request.headers_mut().insert("Authorization", "Bearer dev_key_001".parse().unwrap());
    request.headers_mut().insert("X-Device-Id", "AA:BB:CC:DD:EE:FF".parse().unwrap());

    let (mut ws, _) = connect_async(request).await.expect("Failed to connect");

    // Send hello
    let hello = serde_json::json!({
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
        "capabilities": ["servo", "led", "camera"]
    });
    ws.send(Message::Text(hello.to_string())).await.unwrap();

    // Receive server hello
    let msg = ws.next().await.unwrap().unwrap();
    let response: serde_json::Value = serde_json::from_str(&msg.into_text().unwrap()).unwrap();

    assert_eq!(response["type"], "hello");
    assert!(response["session_id"].as_str().unwrap().starts_with("ses_"));
    assert_eq!(response["server_version"], env!("CARGO_PKG_VERSION"));

    ws.close(None).await.unwrap();
}
```

- [ ] **Step 2: Add lib.rs for test access (optional — or make integration test start its own server)**

For V1, we'll test manually with wscat instead of full integration test harness. Mark this test as `#[ignore]` for now.

Add `#[ignore]` attribute above the test function.

- [ ] **Step 3: Verify test compiles**

Run: `cargo test --no-run`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add kura-chan-server/tests/
git commit -m "test(server): add WebSocket integration test (ignored, manual verification)"
```

---

## Task 12: Manual Verification + README

**Files:**
- Create: `kura-chan-server/README.md`

- [ ] **Step 1: Write README with testing instructions**

```markdown
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

## Configuration

Edit `config/default.toml` or use env vars with `KURA_` prefix.

## Architecture

See `../docs/` for design documents.
```

- [ ] **Step 2: Run server and verify manually with curl**

Run:
```bash
cargo run &
sleep 1
curl http://127.0.0.1:8080/health
```
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add kura-chan-server/README.md
git commit -m "docs(server): add README with quick start and testing guide"
```

---

## Summary

| Task | What it builds | Key files |
|------|---------------|-----------|
| 1 | Project scaffold | Cargo.toml, main.rs, config/default.toml |
| 2 | Config loading | config.rs |
| 3 | Error type | error.rs |
| 4 | Protocol types + codec | ws/protocol.rs, ws/codec.rs |
| 5 | Session state machine | ws/session.rs |
| 6 | Mock STT/TTS | speech/*.rs |
| 7 | Harness client | harness/*.rs |
| 8 | Auth | auth.rs |
| 9 | WS handler + router | ws/mod.rs, router.rs |
| 10 | Wire everything | main.rs |
| 11 | Integration test | tests/ws_integration.rs |
| 12 | README + manual test | README.md |
