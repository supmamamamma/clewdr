use clewdr::{
    self,
    api::AppState,
    config::Config,
    utils::{BANNER, ClewdrError},
};

#[tokio::main]
async fn main() -> Result<(), ClewdrError> {
    // construct a subscriber that prints formatted traces to stdout
    let timer = tracing_subscriber::fmt::time::ChronoLocal::new("%H:%M:%S%.3f".to_string());
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::WARN.into())
        .from_env()
        .unwrap_or_default()
        .add_directive("clewdr=debug".parse().unwrap_or_default());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(timer)
        .compact()
        .init();
    // use that subscriber to process traces emitted after this point
    println!("{}", *BANNER);
    let config = Config::load()?.validate();
    let state = AppState::new(config);
    // TODO: load config from env

    let router = clewdr::api::RouterBuilder::new(state.clone()).build();
    let addr = state.0.config.read().address().to_string();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");
    state.on_listen().await;
    axum::serve(listener, router).await?;
    Ok(())
}
