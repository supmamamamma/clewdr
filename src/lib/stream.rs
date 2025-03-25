use bytes::Bytes;
use futures::{StreamExt, pin_mut};
use parking_lot::RwLock;
use regex::Regex;
use serde_json::{Value, json, to_string_pretty};
use std::mem;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use transform_stream::{AsyncTryStream, Yielder};

use crate::utils::{ClewdrError, DANGER_CHARS, generic_fixes, index_of_any};

#[derive(Clone)]
pub struct ClewdrConfig {
    version: String,
    model: String,
    streaming: bool,
    min_size: usize,
    cancel: CancellationToken,
    prevent_imperson: bool,
}

pub struct ClewdrTransformer {
    config: ClewdrConfig,
    comp_ok: String,
    comp_raw: String,
    comp_all: Vec<String>,
    recv_length: usize,
    hard_censor: bool,
    impersonated: RwLock<bool>,
    error: RwLock<Option<String>>,
    comp_model: String,
}

impl ClewdrTransformer {
    pub fn new(config: ClewdrConfig) -> Self {
        Self {
            config,
            comp_ok: String::new(),
            comp_raw: String::new(),
            comp_all: Vec::new(),
            recv_length: 0,
            hard_censor: false,
            impersonated: RwLock::new(false),
            error: RwLock::new(None),
            comp_model: String::new(),
        }
    }

    fn build(&self, selection: &str) -> String {
        if self.config.streaming {
            let completion = json!({
                "choices": [{
                    "delta": {
                        "content": selection
                    }
                }]
            });
            format!("data: {}\n\n", serde_json::to_string(&completion).unwrap())
        } else {
            let completion = json!({
                "choices": [{
                    "message": {
                        "content": selection
                    }
                }]
            });
            serde_json::to_string(&completion).unwrap()
        }
    }

    fn collect_buf(&mut self) -> String {
        let mut valid: Vec<char> = self.comp_ok.chars().collect();
        let selection: String = valid
            .drain(..self.config.min_size.min(valid.len()))
            .collect();
        self.comp_ok = valid.into_iter().collect();
        selection
    }

    async fn end_early(&self, y: &mut Yielder<Result<String, ClewdrError>>) {
        if self.config.cancel.is_cancelled() {
            return;
        }
        if self.config.streaming {
            y.yield_ok("data: [DONE]\n\n".to_string()).await;
        }
        self.config.cancel.cancel();
    }

    async fn err_json(&self, err: Value, y: &mut Yielder<Result<String, ClewdrError>>) {
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
            self.config.version, self.config.model, code, message
        );
        *self.error.write() = Some(message.clone());
        y.yield_ok(self.build(&message)).await;
        self.end_early(y).await;
    }

    async fn err(&self, err: ClewdrError, y: &mut Yielder<Result<String, ClewdrError>>) {
        warn!("Error: {}", err);
        let message = format!(
            "## {}\n**{} error**:\n{}\n\nFAQ: https://rentry.org/teralomaniac_clewd",
            self.config.version, self.config.model, err
        );
        *self.error.write() = Some(message.clone());
        y.yield_ok(self.build(&message)).await;
        self.end_early(y).await;
    }

    async fn parse_buf(
        &mut self,
        buf: &str,
        y: &mut Yielder<Result<String, ClewdrError>>,
    ) -> Result<(), ClewdrError> {
        let mut delay = false;
        if buf.is_empty() {
            return Ok(());
        }
        if self.config.cancel.is_cancelled() {
            return Ok(());
        }
        let mut parsed = serde_json::from_str::<Value>(buf)?;
        if let Some(error) = parsed.get("error") {
            return Ok(self.err_json(error.clone(), y).await);
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
            parsed.as_object_mut().map(|o| {
                o.insert("completion".to_string(), json!(new_completion));
            });
            self.comp_ok += &new_completion;
            self.comp_all.push(new_completion.clone());
            delay = self.comp_ok.ends_with(DANGER_CHARS.as_slice())
                || new_completion.starts_with(DANGER_CHARS.as_slice());
        }
        if self.config.streaming {
            if delay {
                self.impersonation_check(&self.comp_ok, y).await?;
            }
            while !delay && self.comp_ok.len() >= self.config.min_size {
                let selection = self.collect_buf();
                y.yield_ok(self.build(&selection)).await;
            }
        } else {
            if delay {
                self.impersonation_check(self.comp_all.join("").as_str(), y)
                    .await?;
            }
        }
        Ok(())
    }

    async fn impersonation_check(
        &self,
        reply: &str,
        y: &mut Yielder<Result<String, ClewdrError>>,
    ) -> Result<(), ClewdrError> {
        let fake_any = index_of_any(reply, None);
        if fake_any > -1 {
            *self.impersonated.write() = true;
            if self.config.prevent_imperson {
                let selection = reply[..fake_any as usize].to_string();
                let build = self.build(&selection);
                y.yield_ok(build).await;
                self.end_early(y).await;
            }
        }
        Ok(())
    }

    async fn transform(
        &mut self,
        chunk: Result<Bytes, rquest::Error>,
        y: &mut Yielder<Result<String, ClewdrError>>,
    ) -> Result<(), ClewdrError> {
        let transformer = self;
        let re = Regex::new(r"event: [\w]+\s*|\r")?;
        let chunk = chunk?;

        transformer.recv_length += chunk.len();
        // Decode Bytes to String, assuming UTF-8
        let chunk_str = String::from_utf8(chunk.to_vec())?;
        transformer.comp_raw += &re.replace_all(&chunk_str, "");
        let old_raw = mem::take(&mut transformer.comp_raw);
        let mut substr = old_raw.split("\n\n").collect::<Vec<_>>();
        let last_msg = substr.pop().map(|s| s.to_string());
        transformer.comp_raw = last_msg.unwrap_or_default();

        for i in substr {
            transformer.parse_buf(i, y).await?;
        }
        Ok(())
    }

    async fn flush(
        &mut self,
        y: &mut Yielder<Result<String, ClewdrError>>,
    ) -> Result<(), ClewdrError> {
        // Flush logic
        let transformer = self;
        if !transformer.comp_raw.is_empty() {
            let raw_clone = transformer.comp_raw.clone();
            transformer.parse_buf(raw_clone.as_str(), y).await?;
        }

        if transformer.config.streaming {
            if !transformer.comp_ok.is_empty() {
                y.yield_ok(transformer.build(&transformer.comp_ok)).await;
            }
        } else {
            y.yield_ok(transformer.build(transformer.comp_all.join("").as_str()))
                .await;
        }
        if transformer.comp_all.first().map(|s|s.contains("I apologize, but I will not provide any responses that violate Anthropic's Acceptable Use Policy or could promote harm.")).unwrap_or(false) {
            transformer.hard_censor = true;
        }
        if !transformer.config.cancel.is_cancelled() && transformer.comp_all.is_empty() {
            let err = format!(
                "## {}\n**error**:\n\n```\nReceived no valid replies at all\n```\n",
                transformer.config.version
            );
            y.yield_ok(transformer.build(&err)).await;
        }
        if transformer.config.streaming {
            y.yield_ok("data: [DONE]\n\n".to_string()).await;
        }
        Ok(())
    }

    pub fn transform_stream<S>(
        self,
        input: S,
    ) -> AsyncTryStream<
        String,
        ClewdrError,
        impl std::future::Future<Output = Result<(), ClewdrError>> + Send,
    >
    where
        S: Stream<Item = Result<Bytes, rquest::Error>> + Send + 'static,
    {
        AsyncTryStream::new(move |mut y| async move {
            let mut transformer = self;
            pin_mut!(input);

            while let Some(chunk) = input.next().await {
                if let Err(e) = transformer.transform(chunk, &mut y).await {
                    transformer.err(e, &mut y).await;
                    return Ok(());
                }
            }
            if let Err(e) = transformer.flush(&mut y).await {
                transformer.err(e, &mut y).await;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn stream_test() {
        let cancel = CancellationToken::new();
        let config = ClewdrConfig {
            version: "1.0".to_string(),
            model: "some-model".to_string(),
            streaming: true,
            min_size: 8,
            cancel,
            prevent_imperson: false,
        };

        let input = tokio_stream::iter(vec![
            Ok(Bytes::from("{\"completion\": \"Hello\"}\n\n")),
            Ok(Bytes::from("{\"completion\": \" world\"}\n\n")),
        ]);

        let transformer = ClewdrTransformer::new(config);
        let stream = transformer.transform_stream(input);
        pin_mut!(stream);

        let mut results = String::new();
        while let Some(result) = stream.next().await {
            results += &result.unwrap();
        }
        assert_eq!(
            results,
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello wo\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"rld\"}}]}\n\n\
             data: [DONE]\n\n"
        );
    }
}
