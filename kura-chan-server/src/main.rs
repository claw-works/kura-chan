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
