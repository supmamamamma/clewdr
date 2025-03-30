use std::sync::LazyLock;

use axum::{
    Json,
    body::Body,
    extract::State,
    response::{IntoResponse, Response},
};
use rquest::header::ACCEPT;
use serde::{Deserialize, Serialize, ser::SerializeMap};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::{
    client::{AppendHeaders, SUPER_CLIENT},
    config::UselessReason,
    error::{ClewdrError, check_res_err},
    state::AppState,
    utils::{TIME_ZONE, print_out_json},
};

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
pub struct ReqMessage {
    role: String,
    content: Content,
}

pub static TEST_MESSAGE: LazyLock<ReqMessage> = LazyLock::new(|| ReqMessage {
    role: "user".to_string(),
    content: Content::Array(vec![ContentType::Text {
        text: "Hi".to_string(),
        r#type: "text".to_string(),
    }]),
});

#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
enum Content {
    Array(Vec<ContentType>),
    Raw(String),
}

impl Serialize for Content {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Content::Array(arr) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("array", arr)?;
                map.end()
            }
            Content::Raw(text) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("raw", text)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        if let Some(str) = val.as_str() {
            return Ok(Content::Raw(str.to_string()));
        }
        let vec: Vec<ContentType> = val
            .as_array()
            .ok_or_else(|| serde::de::Error::custom("Expected an array"))?
            .iter()
            .map(|v| {
                serde_json::from_value(v.clone()).map_err(|e| {
                    serde::de::Error::custom(format!("Failed to deserialize ContentType: {}", e))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Content::Array(vec))
    }
}

#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
enum ContentType {
    Text { text: String, r#type: String },
}

impl Serialize for ContentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            ContentType::Text { text, r#type } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("text", text)?;
                map.serialize_entry("type", r#type)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ContentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let map = serde_json::Value::deserialize(deserializer)?;
        let text = map.get("text").and_then(Value::as_str).unwrap_or_default();
        let r#type = map.get("type").and_then(Value::as_str).unwrap_or_default();
        Ok(ContentType::Text {
            text: text.to_string(),
            r#type: r#type.to_string(),
        })
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ResponseBody {
    max_tokens: Option<u64>,
    messages: Vec<ReqMessage>,
    stop_sequences: Vec<String>,
    model: String,
    stream: bool,
    thinking: Option<Thinking>,
    system: Value,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RequestBody {
    files: Vec<Value>,
    model: String,
    rendering_mode: String,
    prompt: String,
    timezone: String,
}

impl From<ResponseBody> for RequestBody {
    fn from(value: ResponseBody) -> Self {
        Self {
            files: vec![],
            model: value.model,
            rendering_mode: "messages".to_string(),
            prompt: value
                .messages
                .into_iter()
                .map(|m| match m.content {
                    Content::Array(arr) => arr
                        .into_iter()
                        .map(|ct| match ct {
                            ContentType::Text { text, .. } => format!("{}: {}", m.role, text),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Content::Raw(text) => format!("{}: {}", m.role, text),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            timezone: TIME_ZONE.to_string(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct Thinking {
    budget_tokens: u64,
    r#type: String,
}

pub async fn api_messages(State(state): State<AppState>, Json(p): Json<ResponseBody>) -> Response {
    match state.try_message(p).await {
        Ok(b) => b.into_response(),
        Err(e) => {
            warn!("Error: {:?}", e);
            e.to_string().into_response()
        }
    }
}

impl AppState {
    async fn try_message(&self, p: ResponseBody) -> Result<Response, ClewdrError> {
        let s = self.0.clone();
        debug!("Messages processed: {:?}", p);
        if !p.stream && p.messages.len() == 1 && p.messages.first() == Some(&TEST_MESSAGE) {
            return Ok(json!({
              "content": [
                {
                  "text": "Hi! My name is Doge.",
                  "type": "text"
                }
              ],
            }
                        )
            .to_string()
            .into_response());
        }
        let uuid = s.conv_uuid.read().clone();
        if let Some(uuid) = uuid {
            self.delete_chat(uuid).await?;
        }
        debug!("Chat deleted");
        *s.conv_uuid.write() = Some(uuid::Uuid::new_v4().to_string());
        *s.conv_depth.write() = 0;
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
        // TODO: 我 log 你的吗，log 都写那么难看
        // finally, send the request

        let body: RequestBody = p.into();
        print_out_json(&body, "4.req.json");
        debug!("Req body processed");
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
        let input_stream = api_res.bytes_stream();
        Ok(Body::from_stream(input_stream).into_response())
    }
}
