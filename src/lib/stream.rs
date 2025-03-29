use axum::response::sse::Event;
use colored::Colorize;
use eventsource_stream::EventStreamError;
use futures::pin_mut;
use parking_lot::RwLock;
use serde_json::{Value, json, to_string_pretty};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::select;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use transform_stream::{AsyncTryStream, Yielder};

use crate::{
    error::{ClewdrError, check_json_err},
    utils::{DANGER_CHARS, clean_json, generic_fixes, index_of_any},
};

#[derive(Clone, Debug)]
pub struct StreamConfig {
    title: String,
    model: String,
    streaming: bool,
    min_size: usize,
    prevent_imperson: bool,
}

impl StreamConfig {
    pub fn new(
        version: &str,
        model: &str,
        streaming: bool,
        min_size: usize,
        prevent_imperson: bool,
    ) -> Self {
        Self {
            title: version.to_string(),
            model: model.to_string(),
            streaming,
            min_size,
            prevent_imperson,
        }
    }
}

#[derive(Debug)]
pub struct ClewdrTransformer {
    config: StreamConfig,
    cancel: CancellationToken,
    ready_string: String,
    completes: Vec<String>,
    recv_length: usize,
    recv_events: AtomicU32,
    emit_events: AtomicU32,
    hard_censor: bool,
    impersonated: AtomicBool,
    error: RwLock<Option<String>>,
    comp_model: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct StreamEventData {
    choices: Vec<StreamEventDelta>,
}

impl StreamEventData {
    fn new(content: String) -> Self {
        Self {
            choices: vec![StreamEventDelta {
                delta: EventContent { content },
            }],
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NonStreamEventData {
    choices: Vec<NonStreamEventMessage>,
}

impl NonStreamEventData {
    fn new(content: String) -> Self {
        Self {
            choices: vec![NonStreamEventMessage {
                message: EventContent { content },
            }],
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct StreamEventDelta {
    delta: EventContent,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NonStreamEventMessage {
    message: EventContent,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct EventContent {
    content: String,
}

impl ClewdrTransformer {
    pub fn new(config: StreamConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
            ready_string: String::with_capacity(1024),
            completes: Vec::with_capacity(1024),
            recv_length: 0,
            recv_events: AtomicU32::new(0),
            emit_events: AtomicU32::new(0),
            hard_censor: false,
            impersonated: AtomicBool::new(false),
            error: RwLock::new(None),
            comp_model: String::new(),
        }
    }

    fn build(&self, selection: &str) -> Event {
        let event = Event::default();
        self.emit_events.fetch_add(1, Ordering::Relaxed);
        if self.config.streaming {
            let data = StreamEventData::new(selection.to_string());
            event.json_data(data).unwrap()
        } else {
            let data = NonStreamEventData::new(selection.to_string());
            event.json_data(data).unwrap()
        }
    }

    fn collect_buf(&mut self) -> String {
        let mut upper = self.config.min_size.min(self.ready_string.len());
        while !&self.ready_string.is_char_boundary(upper) {
            upper += 1;
        }
        self.ready_string.drain(..upper).collect()
    }

    async fn end_early(&self, y: &mut Yielder<Result<Event, ClewdrError>>) {
        if self.config.streaming {
            let event = Event::default();
            y.yield_ok(event.data("[DONE]")).await;
        }
        self.cancel.cancel();
    }

    async fn err_json(&self, err: Value, y: &mut Yielder<Result<Event, ClewdrError>>) {
        warn!("Error: {}", to_string_pretty(&err).unwrap());
        let code = err
            .get("status")
            .or(err.get("code"))
            .or(err.get("type"))
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let message = err
            .get("message")
            .or(err.get("description"))
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let message = format!(
            "## {}\n**{} error**:\n{}\n\n```json\n{}\n```",
            self.config.title, self.config.model, code, message
        );
        *self.error.write() = Some(message.clone());
        y.yield_ok(self.build(&message)).await;
        self.end_early(y).await;
    }

    async fn err(&self, err: ClewdrError, y: &mut Yielder<Result<Event, ClewdrError>>) {
        warn!("Error: {}", err);
        let message = format!(
            "## {}\n**{} error**:\n{}\n\nFAQ: https://rentry.org/teralomaniac_clewd",
            self.config.title, self.config.model, err
        );
        *self.error.write() = Some(message.clone());
        y.yield_ok(self.build(&message)).await;
        self.end_early(y).await;
    }

    async fn parse_buf(&mut self, buf: &str, y: &mut Yielder<Result<Event, ClewdrError>>) {
        let mut delay = false;
        if buf.is_empty() {
            return;
        }
        if self.cancel.is_cancelled() {
            return;
        }
        let buf = clean_json(buf);
        let Ok(mut parsed) = serde_json::from_str::<Value>(buf) else {
            warn!("Failed to parse JSON: {}", buf);
            return;
        };
        if let Some(error) = parsed.get("error") {
            let constructed_error = json!({
                "error": error,
                "status": 500,
            });
            let error = check_json_err(&constructed_error);
            return self.err_json(error, y).await;
        }
        if self.config.model.is_empty() {
            if let Some(model) = parsed.get("model").and_then(|m| m.as_str()) {
                self.comp_model = model.to_string();
            }
        }
        let completion = parsed
            .get("completion")
            .or(parsed.pointer("/delta/text"))
            .or(parsed.pointer("/choices/0/delta/content"))
            .and_then(|c| c.as_str())
            .map(|c| c.to_string());
        if let Some(content) = completion {
            let new_completion = generic_fixes(&content);
            if let Some(o) = parsed.as_object_mut() {
                o.insert("completion".to_string(), json!(new_completion));
            }
            self.ready_string += &new_completion;
            self.completes.push(new_completion.clone());
            delay = self.ready_string.ends_with(DANGER_CHARS.as_slice())
                || new_completion.starts_with(DANGER_CHARS.as_slice());
        }
        if self.config.streaming {
            if delay {
                self.imperson_check(&self.ready_string, y).await;
            }
            while !delay && self.ready_string.len() >= self.config.min_size {
                let selection = self.collect_buf();
                y.yield_ok(self.build(&selection)).await;
            }
        } else if delay {
            self.imperson_check(self.completes.join("").as_str(), y)
                .await;
        }
    }

    async fn imperson_check(&self, reply: &str, y: &mut Yielder<Result<Event, ClewdrError>>) {
        let fake_any = index_of_any(reply, None);
        if fake_any > -1 {
            self.impersonated.store(true, Ordering::Release);
            if self.config.prevent_imperson {
                let selection = &reply[..fake_any as usize];
                let build = self.build(selection);
                y.yield_ok(build).await;
                self.end_early(y).await;
            }
        }
    }

    async fn transform(
        &mut self,
        chunk: Result<eventsource_stream::Event, EventStreamError<rquest::Error>>,
        y: &mut Yielder<Result<Event, ClewdrError>>,
    ) -> Result<(), ClewdrError> {
        let event = chunk.map_err(ClewdrError::EventSourceError)?;
        let data = event.data;
        self.recv_length += data.len();
        self.parse_buf(&data, y).await;
        Ok(())
    }

    async fn flush(&mut self, y: &mut Yielder<Result<Event, ClewdrError>>) {
        // Flush logic
        if self.config.streaming {
            if !self.ready_string.is_empty() {
                y.yield_ok(self.build(&self.ready_string)).await;
            }
        } else {
            y.yield_ok(self.build(self.completes.join("").as_str()))
                .await;
        }
        if self.completes.first().map(|s|s.contains("I apologize, but I will not provide any responses that violate Anthropic's Acceptable Use Policy or could promote harm.")).unwrap_or(false) {
            self.hard_censor = true;
        }
        if !self.cancel.is_cancelled() && self.completes.is_empty() {
            let err = format!(
                "## {}\n**error**:\n\n```\nReceived no valid replies at all\n```\n",
                self.config.title
            );
            y.yield_ok(self.build(&err)).await;
        }
        if self.config.streaming {
            let event = Event::default();
            y.yield_ok(event.data("[DONE]")).await;
        }
    }

    pub fn transform_stream<S>(
        mut self,
        input: S,
    ) -> AsyncTryStream<
        Event,
        ClewdrError,
        impl std::future::Future<Output = Result<(), ClewdrError>> + Send,
    >
    where
        S: Stream<Item = Result<eventsource_stream::Event, EventStreamError<rquest::Error>>>
            + Send
            + 'static,
    {
        AsyncTryStream::new(move |mut y| async move {
            pin_mut!(input);
            loop {
                select! {
                    _ = self.cancel.cancelled() => {
                        error!("Stream cancelled");
                        self.end_early(&mut y).await;
                        return Err(ClewdrError::StreamCancelled(self));
                    }

                    chunk = input.next() => {
                        if let Some(chunk) = chunk {
                            self.recv_events.fetch_add(1, Ordering::Relaxed);
                            if let Err(e) = self.transform(chunk, &mut y).await {
                                error!("Stream error: {}", e);
                                self.err(e, &mut y).await;
                                return Err(ClewdrError::StreamInternalError(self));
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            self.flush(&mut y).await;
            info!(
                "Stream finished. Event received: {}, Input length: {}, Event emit: {}",
                self.recv_events.load(Ordering::Acquire).to_string().blue(),
                format!("{} Chars", self.recv_length).blue(),
                self.emit_events.load(Ordering::Acquire).to_string().blue()
            );
            if self.hard_censor {
                error!("Stream hard censored");
                return Err(ClewdrError::HardCensor(self));
            }
            if self.impersonated.load(Ordering::Acquire) {
                error!("Stream impersonation detected");
                return Err(ClewdrError::Impersonation(self));
            }
            if self.completes.is_empty() || self.recv_length == 0 {
                error!("Stream empty");
                return Err(ClewdrError::EmptyStream(self));
            }
            Ok(())
        })
    }
}
