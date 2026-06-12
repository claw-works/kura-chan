mod config;
mod error;
mod ws;
mod speech;

use config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("kura_chan_server=debug".parse().unwrap()),
        )
        .init();

    let config = Config::load().expect("Failed to load configuration");
    tracing::info!("Config loaded: listening on {}:{}", config.server.host, config.server.port);
}
