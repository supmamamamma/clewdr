use crate::{
    SUPER_CLIENT, TITLE,
    api::AppState,
    stream::{ClewdrTransformer, StreamConfig},
    utils::{
        ClewdrError, ENDPOINT, TEST_MESSAGE, TIME_ZONE, check_res_err, header_ref, print_out_json,
        print_out_text,
    },
};
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response, Sse},
};
use eventsource_stream::EventStream;
use regex::{Regex, RegexBuilder};
use rquest::header::{ACCEPT, COOKIE, ORIGIN, REFERER};
use serde_json::json;
use tracing::{debug, info};

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ClientRequestInfo {
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    max_tokens: Option<i64>,
    #[serde(default)]
    stop: Option<Vec<String>>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    top_k: Option<i64>,
}
impl ClientRequestInfo {
    fn sanitize_client_request(mut self) -> ClientRequestInfo {
        if let Some(ref mut temp) = self.temperature {
            *temp = temp.clamp(0.1, 1.0);
        }
        self
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub customname: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub strip: Option<bool>,
    #[serde(default)]
    pub jailbreak: Option<bool>,
    #[serde(default)]
    pub main: Option<bool>,
    #[serde(default)]
    pub discard: Option<bool>,
    #[serde(default)]
    pub merged: Option<bool>,
    #[serde(default)]
    pub personality: Option<bool>,
    #[serde(default)]
    pub scenario: Option<bool>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct PromptsGroup {
    pub first_user: Option<Message>,
    pub first_system: Option<Message>,
    pub first_assistant: Option<Message>,
    pub last_user: Option<Message>,
    pub last_system: Option<Message>,
    pub last_assistant: Option<Message>,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum RetryStrategy {
    Api,
    Renew,
    RetryRegen,
    CurrentRenew,
    CurrentContinue,
}

impl RetryStrategy {
    pub fn is_current(&self) -> bool {
        matches!(self, Self::CurrentRenew | Self::CurrentContinue)
    }
}

impl PromptsGroup {
    pub fn find(messages: &[Message]) -> PromptsGroup {
        Self {
            first_user: messages.iter().find(|m| m.role == "user").cloned(),
            first_system: messages
                .iter()
                .find(|m| m.role == "system" && m.content != "[Start a new chat]")
                .cloned(),
            first_assistant: messages.iter().find(|m| m.role == "assistant").cloned(),
            last_user: messages.iter().rfind(|m| m.role == "user").cloned(),
            last_system: messages
                .iter()
                .rfind(|m| m.role == "system" && m.content != "[Start a new chat]")
                .cloned(),
            last_assistant: messages.iter().rfind(|m| m.role == "assistant").cloned(),
        }
    }
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: "user".to_string(),
            content: "".to_string(),
            customname: None,
            name: None,
            strip: None,
            jailbreak: None,
            main: None,
            discard: None,
            merged: None,
            personality: None,
            scenario: None,
        }
    }
}

pub async fn completion(
    State(state): State<AppState>,
    _header: HeaderMap,
    Json(payload): Json<ClientRequestInfo>,
) -> Response {
    match state.try_completion(payload).await {
        Ok(b) => b.into_response(),
        Err(e) => {
            info!("Error: {:?}", e);
            e.to_string().into_response()
        }
    }
}

impl AppState {
    async fn try_completion(&self, payload: ClientRequestInfo) -> Result<Response, ClewdrError> {
        // TODO: 3rd key, API key, auth token, etc.
        let s = self.0.as_ref();
        let p = payload.sanitize_client_request();
        *s.model.write() = if s.is_pro.read().is_some() {
            Some(p.model.replace("--force", "").trim().to_string())
        } else {
            s.cookie_model.read().clone()
        };
        if s.uuid_org.read().is_empty() {
            // TODO: more keys
            return Err(ClewdrError::NoValidKey);
        }
        if !*s.changing.read()
            && s.is_pro.read().is_none()
            && *s.model.read() != *s.cookie_model.read()
        {
            self.cookie_changer(None, None);
            self.wait_for_change().await;
        }
        if p.messages.is_empty() {
            return Err(ClewdrError::WrongCompletionFormat);
        }
        print_out_json(&p, "0.messages.json");
        debug!("Messages processed");
        if !p.stream && p.messages.len() == 1 && p.messages.first() == Some(&TEST_MESSAGE) {
            return Ok(json!({
                    "choices":[
                        {
                            "message":{
                                "content": TITLE
                            }
                        }
                    ]
                }
            )
            .to_string()
            .into_response());
        }
        if !p.stream && p.messages.first().map(|f|f.content.starts_with("From the list below, choose a word that best represents a character's outfit description, action, or emotion in their dialogue")).unwrap_or_default() {
            return Ok(
                json!({
                    "choices":[
                        {
                            "message":{
                                "content": "neutral"
                            }
                        }
                    ]
                })
                .to_string().into_response(),
            );
        }
        //  TODO: warn sample config
        if !s.model_list.read().contains(&p.model) && !p.model.contains("claude-") {
            return Err(ClewdrError::InvalidModel(p.model));
        }
        let current_prompts = PromptsGroup::find(&p.messages);
        let previous_prompts = PromptsGroup::find(&s.prev_messages.read());
        debug!("Raw prompts processed");
        let same_prompts = {
            let mut a = p
                .messages
                .iter()
                .filter(|m| m.role != "system")
                .collect::<Vec<_>>();
            a.sort();
            let b = s.prev_messages.read();
            let mut b = b.iter().filter(|m| m.role != "system").collect::<Vec<_>>();
            b.sort();
            a == b
        };
        debug!("Same prompts: {}", same_prompts);
        let same_char_diff_chat = !same_prompts
            && current_prompts.first_system.map(|s| s.content)
                == previous_prompts.first_system.map(|s| s.content)
            && current_prompts.first_user.map(|s| s.content)
                == previous_prompts.first_user.map(|s| s.content);
        let _should_renew = s.config.read().settings.renew_always
            || s.conv_uuid.read().is_none()
            || *s.prev_impersonated.read()
            || (!s.config.read().settings.renew_always && same_prompts)
            || same_char_diff_chat;
        let _retry_regen = s.config.read().settings.retry_regenerate
            && same_prompts
            && s.conv_char.read().is_some();
        if !same_prompts {
            *s.prev_messages.write() = p.messages.clone();
        }
        debug!("Previous prompts processed");

        // TODO: handle api key
        //TODO: handle retry regeneration and not same prompts
        let uuid = s.conv_uuid.read().clone();
        if let Some(uuid) = uuid {
            self.delete_chat(uuid).await?;
        }
        debug!("Chat deleted");
        *s.conv_uuid.write() = Some(uuid::Uuid::new_v4().to_string());
        *s.conv_depth.write() = 0;
        let endpoint = if s.config.read().rproxy.is_empty() {
            ENDPOINT.to_string()
        } else {
            s.config.read().rproxy.clone()
        };
        let endpoint = format!(
            "{}/api/organizations/{}/chat_conversations",
            endpoint,
            s.uuid_org.read()
        );
        let body = json!({
            "uuid": s.conv_uuid.read().as_ref().unwrap(),
            "name":""
        });
        let api_res = SUPER_CLIENT
            .post(endpoint)
            .json(&body)
            .header_append(ORIGIN, ENDPOINT)
            .header_append(REFERER, header_ref(""))
            .header_append(COOKIE, self.header_cookie())
            .send()
            .await?;
        debug!("New conversation created");
        self.update_cookie_from_res(&api_res);
        check_res_err(api_res).await?;
        let r#type = RetryStrategy::Renew;
        // TODO: generate prompts
        let (prompt, _systems) = self.handle_messages(&p.messages, r#type);
        print_out_text(&prompt, "1.prompt.txt");
        debug!("Prompt processed");
        let legacy = {
            let re = RegexBuilder::new(r"claude-([12]|instant)")
                .case_insensitive(true)
                .build()
                .unwrap();
            re.is_match(&p.model)
        };
        debug!("Legacy model: {}", legacy);
        let messages_api = {
            // TODO: third key
            let re = RegexBuilder::new(r"<\|completeAPI\|>")
                .case_insensitive(true)
                .build()
                .unwrap();
            let re2 = Regex::new(r"<\|messagesAPI\|>").unwrap();
            !(legacy || re.is_match(&prompt)) || re2.is_match(&prompt)
        };
        debug!("Messages API: {}", messages_api);
        let messages_log = {
            let re = Regex::new(r"<\|messagesLog\|>").unwrap();
            re.is_match(&prompt)
        };
        debug!("Messages log: {}", messages_log);
        let fusion = {
            let re = Regex::new(r"<\|Fusion Mode\|>").unwrap();
            messages_api && re.is_match(&prompt)
        };
        debug!("Fusion mode: {}", fusion);
        let _wedge = "\r";
        let stop_set = {
            let re = Regex::new(r"<\|stopSet *(\[.*?\]) *\|>").unwrap();
            re.find_iter(&prompt).nth(1)
        };
        let stop_revoke = {
            let re = Regex::new(r"<\|stopRevoke *(\[.*?\]) *\|>").unwrap();
            re.find_iter(&prompt).nth(1)
        };
        let stop_set: Vec<String> = stop_set
            .and_then(|s| serde_json::from_str(s.as_str()).ok())
            .unwrap_or_default();
        debug!("Stop set: {:?}", stop_set);
        let stop_revoke: Vec<String> = stop_revoke
            .and_then(|s| serde_json::from_str(s.as_str()).ok())
            .unwrap_or_default();
        debug!("Stop revoke: {:?}", stop_revoke);
        let stop = stop_set
            .into_iter()
            .chain(p.stop.unwrap_or_default().into_iter())
            .chain(["\n\nHuman:".into(), "\n\nAssistant:".into()])
            .filter(|s| {
                let s = s.trim();
                !s.is_empty() && !stop_revoke.iter().any(|r| r.eq_ignore_ascii_case(s))
            })
            .collect::<Vec<_>>();
        debug!("Stop seq: {:?}", stop);
        // TODO: Api key
        let prompt = if s.config.read().settings.xml_plot {
            self.xml_plot(
                prompt,
                Some(
                    legacy
                        && !s
                            .model
                            .read()
                            .as_ref()
                            .map(|m| m.contains("claude-2.1"))
                            .unwrap_or_default(),
                ),
            )
        } else {
            // TODO: handle api key
            unimplemented!()
        };
        print_out_text(&prompt, "2.xml.txt");
        debug!("XML regex processed");
        let mut pr = self.pad_txt(prompt);
        print_out_text(&pr, "3.pad.txt");
        debug!("Pad txt processed");
        // TODO: 我 log 你的吗，log 都写那么难看
        // panic!("log");
        // finally, send the request
        // TODO: handle retry regeneration
        let mut attach = json!([]);
        if s.config.read().settings.prompt_experiments {
            let splitted = pr
                .split("\n\nPlainPrompt:")
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let new_p = splitted[0].to_string();
            attach = json!([{
                "extracted_content": new_p,
                "file_name": "paste.txt",
                "file_type": "txt",
                "file_size": new_p.len(),
            }]);
            pr = if r#type == RetryStrategy::Renew {
                s.config.read().prompt_experiment_first.clone()
            } else {
                s.config.read().prompt_experiment_next.clone()
            };
            if splitted.len() > 1 {
                pr += splitted[1].as_str();
            }
        }

        let mut body = json!({
            "attachments": attach,
            "files": [],
            "model": if s.is_pro.read().as_ref().is_some() {
                Some(s.model.read().as_ref().cloned().unwrap_or_default())
            } else {
                None
            },
            "rendering_mode": "raw",
            // TODO: pass parameters
            "prompt": pr,
            "timezone": TIME_ZONE,
        });
        if s.config.read().settings.pass_params {
            if let Some(mt) = p.max_tokens {
                body["max_tokens_to_sample"] = json!(mt)
            }
            if let Some(tk) = p.max_tokens {
                body["top_k"] = json!(tk)
            }
            if let Some(tp) = p.top_p {
                body["top_p"] = json!(tp)
            }
            // body["stop_sequences"] = json!(stop);
            // body["temperature"] = json!(p.temperature);
        }
        print_out_json(&body, "4.req.json");
        debug!("Req body processed");
        let endpoint = if s.config.read().api_rproxy.is_empty() {
            ENDPOINT.to_string()
        } else {
            s.config.read().api_rproxy.clone()
        };
        let endpoint = format!(
            "{}/api/organizations/{}/chat_conversations/{}/completion",
            endpoint,
            s.uuid_org.read(),
            s.conv_uuid.read().as_ref().cloned().unwrap_or_default()
        );

        let api_res = SUPER_CLIENT
            .post(endpoint)
            .json(&body)
            .header_append(ORIGIN, ENDPOINT)
            .header_append(REFERER, header_ref(""))
            .header_append(ACCEPT, "text/event-stream")
            .header_append(COOKIE, self.header_cookie())
            .send()
            .await?;
        self.update_cookie_from_res(&api_res);
        let api_res = check_res_err(api_res).await?;
        let trans = ClewdrTransformer::new(StreamConfig::new(
            TITLE,
            s.model
                .read()
                .as_ref()
                .cloned()
                .unwrap_or_default()
                .as_str(),
            p.stream,
            s.config.read().buffer_size as usize,
            s.config.read().settings.prevent_imperson,
        ));
        let input_stream = api_res.bytes_stream();
        let event_stream = EventStream::new(input_stream);
        let output_stream = trans.transform_stream(event_stream);
        Ok(Sse::new(output_stream).into_response())
    }
}
