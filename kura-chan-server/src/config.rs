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
    #[serde(default = "default_region")]
    pub region: String,
    /// AgentCore harness ARN. Keep out of committed config; set via env
    /// `HARNESS_ARN` (account-specific). See Config::load.
    #[serde(default)]
    pub harness_arn: String,
}

fn default_region() -> String {
    "us-west-2".to_string()
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
    /// Idle gap (seconds) after which a new conversation session is started.
    #[serde(default = "default_idle_new_session")]
    pub idle_new_session_secs: u64,
}

fn default_idle_new_session() -> u64 {
    7200
}

impl Config {
    pub fn load() -> Result<Self, figment::Error> {
        let mut cfg: Config = Figment::new()
            .merge(Toml::file("config/default.toml"))
            .merge(Env::prefixed("KURA_").split("_"))
            .extract()?;
        // Explicit env overrides for AWS settings (kept out of committed config).
        // figment's split("_") can't bind underscored fields like `harness_arn`.
        if let Ok(arn) = std::env::var("HARNESS_ARN") {
            cfg.aws.harness_arn = arn;
        }
        if let Ok(region) = std::env::var("AWS_REGION") {
            cfg.aws.region = region;
        }
        if cfg.aws.harness_arn.is_empty() {
            tracing::warn!(
                "harness_arn is empty; set the HARNESS_ARN environment variable"
            );
        }
        Ok(cfg)
    }
}
