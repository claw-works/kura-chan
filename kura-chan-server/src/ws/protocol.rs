use serde::{Deserialize, Serialize};

// === Device → Server ===

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello(ClientHello),
    ToolResult(ToolResult),
    Status(DeviceStatus),
    Event(DeviceEvent),
}

#[derive(Debug, Deserialize)]
pub struct DeviceEvent {
    pub kind: String, // e.g. "head_pat"
}

#[derive(Debug, Deserialize)]
pub struct DeviceStatus {
    pub battery: Option<i32>,
    pub charging: Option<bool>,
    pub volume: Option<i32>,
    /// device-reported current appearance selection {hair,costume,blush,glasses}
    pub appearance: Option<serde_json::Value>,
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
    /// Actuator command for the device (volume / led / turn).
    Control(ControlMessage),
    /// Server-authoritative state pushed to the device (growth + appearance).
    Sync(SyncState),
    /// Sent after all streamed audio for a reply has been pushed.
    SpeakDone,
}

#[derive(Debug, Serialize)]
pub struct SyncState {
    pub gender: String,
    pub appearance: serde_json::Value,
    pub level: i32,
    pub xp: i32,
    pub xp_need: i32,
    pub bond: i32,
    pub energy: i32,
}

#[derive(Debug, Serialize)]
pub struct ControlMessage {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
