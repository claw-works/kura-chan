use figment::{Figment, providers::{Format, Toml, Env}};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub aws: AwsConfig,
    pub agent: AgentConfig,
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

/// Agent persona / behaviour, injected as the system prompt on every
/// invoke_harness call (overrides the harness default).
#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SpeechConfig {
    pub stt_provider: String,
    pub tts_provider: String,
    /// Volcengine ASR resource id, e.g. "volc.seedasr.sauc.concurrent"
    #[serde(default)]
    pub volc_asr_resource_id: String,
    /// Volcengine TTS resource id, e.g. "seed-tts-2.0"
    #[serde(default)]
    pub volc_tts_resource_id: String,
    /// Volcengine TTS voice/speaker id, e.g. "zh_female_sajiaoxuemei_uranus_bigtts"
    #[serde(default)]
    pub volc_tts_voice: String,
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
