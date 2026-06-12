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
