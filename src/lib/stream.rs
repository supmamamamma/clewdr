use bytes::Bytes;
use futures::{StreamExt, pin_mut};
use regex::Regex;
use serde_json::{Value, json};
use std::mem;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use transform_stream::AsyncTryStream;

use crate::utils::ClewdrError;

#[derive(Clone)]
pub struct ClewdConfig {
    version: String,
    model: String,
    streaming: bool,
    min_size: usize,
    cancel: CancellationToken,
}

pub struct ClewdTransformer {
    config: ClewdConfig,
    comp_ok: String,
    comp_raw: String,
    comp_all: Vec<String>,
    recv_length: usize,
    ended: bool,
    hard_censor: bool,
    impersonated: bool,
    error: Option<String>,
    comp_model: String,
}

impl ClewdTransformer {
    pub fn new(config: ClewdConfig) -> Self {
        Self {
            config,
            comp_ok: String::new(),
            comp_raw: String::new(),
            comp_all: Vec::new(),
            recv_length: 0,
            ended: false,
            hard_censor: false,
            impersonated: false,
            error: None,
            comp_model: String::new(),
        }
    }

    fn build(&self, selection: String) -> Result<String, ClewdrError> {
        if self.config.streaming {
            let completion = json!({
                "choices": [{
                    "delta": {
                        "content": selection
                    }
                }]
            });
            Ok(format!("data: {}\n\n", serde_json::to_string(&completion)?))
        } else {
            let completion = json!({
                "choices": [{
                    "message": {
                        "content": selection
                    }
                }]
            });
            Ok(serde_json::to_string(&completion)?)
        }
    }

    fn collect_buf(&mut self) -> String {
        let mut valid: Vec<char> = self.comp_ok.chars().collect();
        let selection: String = valid
            .drain(0..self.config.min_size.min(valid.len()))
            .collect();
        self.comp_ok = valid.into_iter().collect();
        selection
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
        S: Stream<Item = Result<Bytes, ClewdrError>> + Send + 'static,
    {
        AsyncTryStream::new(move |mut y| async move {
            let mut transformer = self;
            pin_mut!(input);

            let re = Regex::new(r"event: [\w]+\s*|\r").unwrap();

            while let Some(chunk) = input.next().await {
                let chunk = chunk?;
                if transformer.ended || transformer.config.cancel.is_cancelled() {
                    continue;
                }

                transformer.recv_length += chunk.len();
                // Decode Bytes to String, assuming UTF-8
                let chunk_str = match String::from_utf8(chunk.to_vec()) {
                    Ok(s) => s,
                    Err(e) => {
                        let err_msg = format!("UTF-8 decoding error: {}", e);
                        transformer.error = Some(err_msg.clone());
                        transformer.ended = true;
                        y.yield_ok(transformer.build(err_msg)?).await;
                        return Ok(());
                    }
                };
                transformer.comp_raw += &re.replace_all(&chunk_str, "");
                let old_raw = mem::take(&mut transformer.comp_raw);
                let mut substr = old_raw.split("\n\n").collect::<Vec<_>>();
                let last_msg = substr.pop().map(|s| s.to_string());
                transformer.comp_raw = last_msg.unwrap_or_default();

                for i in substr {
                    if let Ok(parsed) = serde_json::from_str::<Value>(i) {
                        if let Some(error) = parsed.get("error") {
                            let err_msg = format!(
                                "## {}\n**{} error**:\n{}\n\n```\n{}\n```\n\nFAQ: https://rentry.org/teralomaniac_clewd",
                                transformer.config.version,
                                transformer.config.model,
                                error
                                    .get("status")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("unknown"),
                                error
                                    .get("message")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("unknown")
                            );
                            transformer.error = Some(err_msg.clone());
                            transformer.ended = true;
                            y.yield_ok(transformer.build(err_msg)?).await;
                            return Ok(());
                        }

                        if transformer.comp_model.is_empty() {
                            if let Some(model) = parsed.get("model").and_then(|m| m.as_str()) {
                                transformer.comp_model = model.to_string();
                            }
                        }

                        if let Some(content) = parsed
                            .get("completion")
                            .or(parsed.get("delta").and_then(|d| d.get("text")))
                            .or(parsed
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("delta"))
                                .and_then(|d| d.get("content")))
                            .and_then(|c| c.as_str())
                        {
                            let content = content.to_string();
                            transformer.comp_ok += &content;
                            transformer.comp_all.push(content);

                            if transformer.config.streaming {
                                while transformer.comp_ok.len() >= transformer.config.min_size {
                                    let selection = transformer.collect_buf();
                                    y.yield_ok(transformer.build(selection)?).await;
                                }
                            }
                        }
                    }
                }
            }

            // Flush logic
            if !transformer.comp_raw.is_empty() {
                transformer.comp_ok += &transformer.comp_raw;
                transformer.comp_raw.clear();
            }

            if transformer.config.streaming {
                if !transformer.comp_ok.is_empty() {
                    y.yield_ok(transformer.build(transformer.comp_ok.clone())?)
                        .await;
                    transformer.comp_ok.clear();
                }
                if !transformer.ended && !transformer.config.cancel.is_cancelled() {
                    y.yield_ok("data: [DONE]\n\n".to_string()).await;
                }
            } else {
                let full_content = transformer.comp_all.join("");
                if full_content.is_empty() && !transformer.ended {
                    let err = format!(
                        "## {}\n**error**:\n\n```\nReceived no valid replies at all\n```\n\nFAQ: https://rentry.org/teralomaniac_clewd",
                        transformer.config.version
                    );
                    transformer.error = Some(err.clone());
                    y.yield_ok(transformer.build(err)?).await;
                } else {
                    if full_content.contains("I apologize, but I will not provide") {
                        transformer.hard_censor = true;
                    }
                    y.yield_ok(transformer.build(full_content)?).await;
                }
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
        let config = ClewdConfig {
            version: "1.0".to_string(),
            model: "some-model".to_string(),
            streaming: true,
            min_size: 8,
            cancel,
        };

        let input = tokio_stream::iter(vec![
            Ok(Bytes::from("{\"completion\": \"Hello\"}\n\n")),
            Ok(Bytes::from("{\"completion\": \" world\"}\n\n")),
        ]);

        let transformer = ClewdTransformer::new(config);
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
