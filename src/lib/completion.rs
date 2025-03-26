use crate::{
    SUPER_CLIENT,
    api::{AppState, InnerState},
    stream::{ClewdrConfig, ClewdrTransformer},
};
use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::HeaderMap,
};
use bytes::Bytes;
use futures::pin_mut;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};
use tokio_util::sync::CancellationToken;
use tracing::info;

pub async fn stream_example(
    State(state): State<AppState>,
    header: HeaderMap,
    Json(payload): Json<Value>,
) -> Body {
    // Create a channel for streaming response chunks to the client
    let (tx, rx) = mpsc::channel::<Result<Bytes, axum::Error>>(32);

    // Configure the transformer
    let config = ClewdrConfig::new("xx", "pro", true, 8, true);
    let trans = ClewdrTransformer::new(config);

    // Perform the external request
    let super_res = SUPER_CLIENT
        .get("https://api.claude.ai")
        .send()
        .await
        .unwrap(); // In production, handle this error gracefully
    let input_stream = super_res.bytes_stream();

    // Spawn a task to handle the streaming transformation
    tokio::spawn(async move {
        let output_stream = trans.transform_stream(input_stream);
        pin_mut!(output_stream);

        while let Some(result) = output_stream.next().await {
            // Simulate expensive work (optional, adjust as needed)
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Send the chunk to the client
            let chunk = Bytes::from(result.unwrap()); // Convert String to Bytes
            if tx.send(Ok(chunk)).await.is_err() {
                info!("Client disconnected, cancelling task");
                break;
            }
        }
    });

    // Return the streaming body
    let response_stream = ReceiverStream::new(rx);
    Body::from_stream(response_stream)
}

pub async fn completion(
    State(state): State<AppState>,
    header: HeaderMap,
    Json(payload): Json<Value>,
) {
    let state = state.0;
    let temp = payload["temperature"].clone();
    // if temp is not f64 or within 0.1 to 1.0, set it to Null
    let temp = if temp.is_f64() && (0.1..=1.0).contains(&temp.as_f64().unwrap()) {
        temp
    } else {
        Value::Null
    };
    let message = payload["messages"].clone();
    let auth = header
        .get("Authorization")
        .unwrap()
        .to_str()
        .unwrap_or_default();
    let third_key = auth
        .trim_start_matches("oaiKey:")
        .trim_start_matches("3rdKey:");
    let is_oai_api = auth.matches("oaiKey:").count() > 0;
    let force_model = payload["model"]
        .as_str()
        .unwrap_or_default()
        .contains("--force");
    let api_keys = third_key.split(",").map(|s| s.trim()).collect::<Vec<_>>();
    // TODO: validate api keys
    let model = if !api_keys.is_empty() || force_model || state.is_pro.read().is_some() {
        let m = payload["model"]
            .as_str()
            .unwrap_or_default()
            .replace("--force", "");
        m.trim().to_string()
    } else {
        state.cookie_model.read().clone()
    };
    let max_tokens_to_sample = payload["max_tokens"].as_u64().unwrap_or(0);
    let stop_sequence = payload["stop"].as_str().unwrap_or_default();
    let top_p = payload["top_p"].clone();
    let top_k = payload["top_k"].clone();
    let config = &state.config.read();
    if api_keys.is_empty()
        && (!config.proxy_password.is_empty()
            && auth != format!("Bearer {}", config.proxy_password)
            || state.uuid_org.read().is_empty())
    {
        let msg = if state.uuid_org.read().is_empty() {
            "No cookie available or apiKey format wrong"
        } else {
            "proxy_password Wrong"
        };
        panic!("{}", msg);
    } else if !*state.changing.read()
        && !api_keys.is_empty()
        && state.is_pro.read().is_none()
        && model != *state.cookie_model.read()
    {
        panic!("No cookie available");
    }
    // TODO
}
