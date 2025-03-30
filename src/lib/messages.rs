use std::{fmt::Debug, mem, sync::LazyLock};

use axum::{
    Json,
    body::Body,
    extract::State,
    response::{IntoResponse, Response},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use futures::future::join_all;
use rquest::{
    header::ACCEPT,
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::{
    client::{AppendHeaders, SUPER_CLIENT},
    config::UselessReason,
    error::{ClewdrError, check_res_err},
    state::AppState,
    types::message::{ContentBlock, ImageSource, Message, MessageContent, Role},
    utils::{TIME_ZONE, print_out_json},
};

pub static TEST_MESSAGE: LazyLock<Message> = LazyLock::new(|| {
    Message::new_blocks(
        Role::User,
        vec![ContentBlock::Text {
            text: "Hi".to_string(),
        }],
    )
});

#[derive(Deserialize, Serialize, Debug)]
pub struct RequestBody {
    files: Vec<String>,
    model: String,
    rendering_mode: String,
    prompt: String,
    timezone: String,
    #[serde(skip)]
    images: Vec<ImageSource>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ClientRequestBody {
    max_tokens: Option<u64>,
    messages: Vec<Message>,
    stop_sequences: Vec<String>,
    model: String,
    #[serde(default)]
    stream: bool,
    thinking: Option<Thinking>,
    #[serde(default)]
    system: Value,
}

impl From<ClientRequestBody> for RequestBody {
    fn from(value: ClientRequestBody) -> Self {
        let mut images = vec![];
        let prompt = value
            .messages
            .iter()
            .map(|m| {
                let r = match m.role {
                    Role::User => "User: ",
                    Role::Assistant => "Assistant: ",
                };
                let c = match &m.content {
                    MessageContent::Text { content } => content.clone(),
                    MessageContent::Blocks { content } => content
                        .iter()
                        .map_while(|b| match b {
                            ContentBlock::Text { text } => Some(text.trim().to_string()),
                            ContentBlock::Image { source } => {
                                images.push(source.clone());
                                None
                            }
                            _ => None,
                        })
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                format!("{}{}", r, c)
            })
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            files: vec![],
            model: value.model,
            rendering_mode: "messages".to_string(),
            prompt,
            timezone: TIME_ZONE.to_string(),
            images,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct Thinking {
    budget_tokens: u64,
    r#type: String,
}

pub async fn api_messages(
    State(state): State<AppState>,
    Json(p): Json<ClientRequestBody>,
) -> Response {
    match state.try_message(p).await {
        Ok(b) => b.into_response(),
        Err(e) => {
            warn!("Error: {:?}", e);
            e.to_string().into_response()
        }
    }
}

impl AppState {
    async fn try_message(&self, p: ClientRequestBody) -> Result<Response, ClewdrError> {
        let s = self.0.clone();
        print_out_json(&p, "0.req.json");

        // Check if the request is a test message
        if !p.stream && p.messages.len() == 1 && p.messages[0] == *TEST_MESSAGE {
            return Ok(json!({
                "content": [
                    {
                        "text": "Hi! My name is Doge.",
                        "type": "text"
                    }
                ],
            })
            .to_string()
            .into_response());
        }

        // delete the previous conversation if it exists
        self.delete_chat().await?;
        debug!("Chat deleted");

        // Create a new conversation
        *s.conv_uuid.write() = Some(uuid::Uuid::new_v4().to_string());
        let endpoint = s.config.read().endpoint("");
        let endpoint = format!(
            "{}/api/organizations/{}/chat_conversations",
            endpoint,
            s.uuid_org.read()
        );
        let mut body = json!({
            "uuid": s.conv_uuid.read().as_ref().unwrap(),
            "name":""
        });
        if p.thinking.is_some() {
            body["paprika_mode"] = "extended".into();
            body["model"] = p.model.clone().into();
        }
        let api_res = SUPER_CLIENT
            .post(endpoint)
            .json(&body)
            .append_headers("", &self.header_cookie()?)
            .send()
            .await?;
        debug!("New conversation created");
        self.update_cookie_from_res(&api_res);
        check_res_err(api_res).await?;

        // prepare the request
        let mut body: RequestBody = p.into();
        // check images
        let images = mem::take(&mut body.images);

        // upload images
        let fut = images
            .into_iter()
            .map_while(|img| {
                if img.type_ != "base64" {
                    warn!("Image type is not base64");
                    return None;
                }
                let Ok(bytes) = BASE64_STANDARD.decode(img.data.as_bytes()) else {
                    warn!("Failed to decode base64 image");
                    return None;
                };
                let file_name = match img.media_type.as_str() {
                    "image/png" => "image.png",
                    "image/jpeg" => "image.jpg",
                    "image/gif" => "image.gif",
                    "image/webp" => "image.webp",
                    "application/pdf" => "document.pdf",
                    _ => "file",
                };
                let part = Part::bytes(bytes).file_name(file_name);
                let form = Form::new().part("file", part);

                let endpoint = format!("https://claude.ai/api/{}/upload", s.uuid_org.read(),);
                Some(
                    SUPER_CLIENT
                        .post(endpoint)
                        .append_headers("new", self.header_cookie().ok()?)
                        .header_append("anthropic-client-platform", "web_claude_ai")
                        .multipart(form)
                        .send(),
                )
            })
            .collect::<Vec<_>>();

        // get upload responses
        let fut = join_all(fut)
            .await
            .into_iter()
            .map_while(|r| {
                r.inspect_err(|e| {
                    warn!("Failed to upload image: {:?}", e);
                })
                .ok()
            })
            .map(|r| async {
                let json = r
                    .json::<Value>()
                    .await
                    .inspect_err(|e| {
                        warn!("Failed to parse image response: {:?}", e);
                    })
                    .ok()?;
                Some(json["file_uuid"].as_str()?.to_string())
            })
            .collect::<Vec<_>>();

        let files = join_all(fut)
            .await
            .into_iter()
            .filter_map(|r| r)
            .collect::<Vec<_>>();
        body.files = files;

        // file processed
        print_out_json(&body, "4.req.json");
        let endpoint = s.config.read().endpoint("");
        let endpoint = format!(
            "{}/api/organizations/{}/chat_conversations/{}/completion",
            endpoint,
            s.uuid_org.read(),
            s.conv_uuid.read().as_ref().cloned().unwrap_or_default()
        );

        let api_res = SUPER_CLIENT
            .post(endpoint)
            .json(&body)
            .append_headers("", self.header_cookie()?)
            .header_append(ACCEPT, "text/event-stream")
            .send()
            .await?;
        self.update_cookie_from_res(&api_res);
        let api_res = check_res_err(api_res).await.inspect_err(|e| {
            if let ClewdrError::TooManyRequest(_, i) = e {
                self.cookie_rotate(UselessReason::Temporary(*i));
            }
        })?;

        // stream the response
        let input_stream = api_res.bytes_stream();
        Ok(Body::from_stream(input_stream).into_response())
    }
}
