use clewdr::{
    self,
    api::AppState,
    config::Config,
    utils::{BANNER, ClewdrError},
};

#[tokio::main]
async fn main() -> Result<(), ClewdrError> {
    // construct a subscriber that prints formatted traces to stdout
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    // use that subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber).unwrap();
    println!("{}", *BANNER);
    let config = Config::load()?.validate();
    let state = AppState::new(config);
    // TODO: load config from env

    let router = clewdr::api::RouterBuilder::new(state.clone()).build();
    let listener = tokio::net::TcpListener::bind(state.0.config.read().address())
        .await
        .expect("Failed to bind to address");
    state.on_listen().await;
    axum::serve(listener, router).await.unwrap();

    Ok(())
}
