mod agent_loop;
mod api;
mod assets;
mod auth;
mod config;
mod db;
mod error;
mod harness;
mod registry;
mod router;
mod speech;
mod tasks;
mod workflows;
mod ws;

use std::sync::Arc;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::harness::HarnessClient;
use crate::speech::mock_stt::MockStt;
use crate::speech::mock_tts::MockTts;
use crate::speech::volc::{VolcStt, VolcTts};
use crate::speech::{SpeechToText, TextToSpeech};
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

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://dev:dev@localhost:5432/dev".to_string());
    let db = db::connect(&database_url).await.expect("DB connect/migrate failed");
    db::seed_dev(&db).await.ok();
    tracing::info!("Postgres connected + migrated");

    let harness = HarnessClient::new(&config.aws).await;
    tracing::info!("Harness client initialized");

    let stt = build_stt(&config);
    let tts = build_tts(&config);

    let registry = std::sync::Arc::new(registry::SessionRegistry::new());
    let task_store = std::sync::Arc::new(tasks::TaskStore::load(
        std::path::PathBuf::from("tasks.json"),
    ));
    let workflow_store = std::sync::Arc::new(workflows::WorkflowStore::load(
        std::path::PathBuf::from("workflows.json"),
    ));

    let state = Arc::new(AppState {
        config: config.clone(),
        harness,
        stt,
        tts,
        canned: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        registry,
        task_store,
        workflow_store,
        db,
    });

    // Agent loop: slow heartbeat + scheduled-task scheduler (proactive push).
    tokio::spawn(agent_loop::run(state.clone()));

    let app = router::create_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");
    tracing::info!("Kura-chan server listening on {}", addr);

    axum::serve(listener, app).await.expect("Server error");
}

fn build_stt(config: &Config) -> Box<dyn SpeechToText> {
    match config.speech.stt_provider.as_str() {
        "volc" => match std::env::var("VOLC_API_KEY") {
            Ok(key) if !key.is_empty() => {
                tracing::info!("STT: Volcengine ASR");
                Box::new(VolcStt::new(key, config.speech.volc_asr_resource_id.clone()))
            }
            _ => {
                tracing::warn!("VOLC_API_KEY not set; STT falling back to mock");
                Box::new(MockStt)
            }
        },
        _ => {
            tracing::info!("STT: mock");
            Box::new(MockStt)
        }
    }
}

fn build_tts(config: &Config) -> Box<dyn TextToSpeech> {
    match config.speech.tts_provider.as_str() {
        "volc" => match std::env::var("VOLC_API_KEY") {
            Ok(key) if !key.is_empty() => {
                tracing::info!("TTS: Volcengine TTS");
                Box::new(VolcTts::new(
                    key,
                    config.speech.volc_tts_resource_id.clone(),
                    config.speech.volc_tts_voice.clone(),
                ))
            }
            _ => {
                tracing::warn!("VOLC_API_KEY not set; TTS falling back to mock");
                Box::new(MockTts)
            }
        },
        _ => {
            tracing::info!("TTS: mock");
            Box::new(MockTts)
        }
    }
}
