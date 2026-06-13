use std::sync::Arc;
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

pub struct Session {
    pub session_id: String,
    pub device_id: String,
    pub state: SessionState,
    pub audio_buffer: Vec<u8>,
    pub config: Arc<Config>,
    pub battery: Option<i32>,
    pub charging: Option<bool>,
    pub volume: Option<i32>,
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
            battery: None,
            charging: None,
            volume: None,
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
        vec![
            SessionEvent::SendJson(ServerMessage::State(StateChange {
                state: "speaking".into(),
            })),
            SessionEvent::SendJson(ServerMessage::Response(response)),
        ]
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

    pub fn handle_status(&mut self, status: DeviceStatus) -> Vec<SessionEvent> {
        if let Some(b) = status.battery { self.battery = Some(b); }
        if let Some(c) = status.charging { self.charging = Some(c); }
        if let Some(v) = status.volume { self.volume = Some(v); }
        tracing::debug!(battery = ?self.battery, charging = ?self.charging, volume = ?self.volume, "Device status");
        vec![]
    }

    /// Build the user message sent to the agent, optionally prefixed with a
    /// concise device-state line so the agent can answer battery/volume queries.
    pub fn build_user_message(&self, utterance: &str) -> String {
        let mut ctx = String::new();
        if self.battery.is_some() || self.volume.is_some() {
            ctx.push_str("[设备状态]");
            if let Some(b) = self.battery {
                ctx.push_str(&format!(" 电量{}%", b));
                if self.charging == Some(true) { ctx.push_str("(充电中)"); }
            }
            if let Some(v) = self.volume {
                ctx.push_str(&format!(" 音量{}%", v));
            }
            ctx.push('\n');
        }
        format!("{}{}", ctx, utterance)
    }
}
