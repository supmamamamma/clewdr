use clewdr::{self, config::Config, error::ClewdrError, state::AppState, utils::BANNER};
use colored::Colorize;
use const_format::formatc;

#[tokio::main]
async fn main() -> Result<(), ClewdrError> {
    // construct a subscriber that prints formatted traces to stdout
    let timer = tracing_subscriber::fmt::time::ChronoLocal::new("%H:%M:%S%.3f".to_string());
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::Level::DEBUG.into())
        .from_env()
        .unwrap_or_default();
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(timer)
        .compact()
        .init();
    // use that subscriber to process traces emitted after this point
    println!("{}", *BANNER);
    let config = Config::load()?;
    // TODO: load config from env

    // get time now
    const TITLE: &str = formatc!(
        "Clewdr v{} by {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_AUTHORS")
    );
    println!("{}", TITLE.blue());
    println!("Listening on {}", config.address().green());
    // println!("Config:\n{:?}", config);
    // TODO: Local tunnel

    let state = AppState::new(config);
    let router = clewdr::router::RouterBuilder::new(state.clone()).build();
    let addr = state.0.config.read().address().to_string();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    state.bootstrap().await;
    axum::serve(listener, router).await?;
    Ok(())
}
