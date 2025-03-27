use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, options, post},
};
use colored::Colorize;
use const_format::{concatc, formatc};
use parking_lot::RwLock;
use regex::{Regex, RegexBuilder};
use rquest::Response;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{spawn, time::timeout};
use tracing::error;

use crate::{
    NORMAL_CLIENT, SUPER_CLIENT,
    completion::{Message, completion, stream_example},
    config::{Config, UselessCookie, UselessReason},
    utils::{ClewdrError, ENDPOINT, JsBool, MODELS, check_res_err, header_ref, print_out_json},
};

impl RouterBuilder {
    pub fn new(state: AppState) -> Self {
        Self {
            inner: Router::new()
                .route("/v1/test", post(stream_example))
                .route("/v1/models", get(get_models))
                .route("/v1/chat/completions", post(completion))
                .route("/v1/complete", post(api_complete))
                .route("/v1", options(api_options))
                .route("/", options(api_options))
                .fallback(api_fallback)
                .with_state(state),
        }
    }

    pub fn build(self) -> Router {
        self.inner
    }
}

pub struct RouterBuilder {
    inner: Router,
}

#[derive(Default)]
pub struct InnerState {
    pub config: RwLock<Config>,
    pub model_list: RwLock<Vec<String>>,
    pub is_pro: RwLock<Option<String>>,
    pub cookie_model: RwLock<Option<String>>,
    pub uuid_org: RwLock<String>,
    pub changing: RwLock<bool>,
    pub change_flag: RwLock<usize>,
    pub current_index: RwLock<usize>,
    pub first_login: RwLock<bool>,
    pub timestamp: RwLock<i64>,
    pub change_times: RwLock<usize>,
    pub total_times: usize,
    pub model: RwLock<Option<String>>,
    pub cookies: RwLock<HashMap<String, String>>,
    pub uuid_org_array: RwLock<Vec<String>>,
    pub conv_uuid: RwLock<Option<String>>,
    pub conv_char: RwLock<Option<String>>,
    pub conv_depth: RwLock<i64>,
    pub prev_messages: RwLock<Vec<Message>>,
    pub prev_impersonated: RwLock<bool>,
    pub regex_log: RwLock<String>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<InnerState>);

impl AppState {
    pub fn new(config: Config) -> Self {
        let total_times = config.cookie_array.len();
        let m = InnerState {
            config: RwLock::new(config),
            first_login: RwLock::new(true),
            total_times,
            ..Default::default()
        };
        let m = Arc::new(m);
        AppState(m)
    }

    pub fn update_cookie_from_res(&self, res: &Response) {
        res.headers()
            .get("set-cookie")
            .and_then(|h| h.to_str().ok())
            .map(|s| self.update_cookies(s));
    }

    pub fn update_cookies(&self, str: &str) {
        let str = str.split("\n").to_owned().collect::<Vec<_>>().join("");
        if str.is_empty() {
            return;
        }
        let re1 = Regex::new(r";\s?").unwrap();
        let re2 = RegexBuilder::new(r"^(path|expires|domain|HttpOnly|Secure|SameSite)[=;]*")
            .case_insensitive(true)
            .build()
            .unwrap();
        let re3 = Regex::new(r"^(.*?)=\s*(.*)").unwrap();
        re1.split(&str)
            .filter(|s| !re2.is_match(s) && !s.is_empty())
            .for_each(|s| {
                let caps = re3.captures(s);
                if let Some(caps) = caps {
                    let key = caps[1].to_string();
                    let value = caps[2].to_string();
                    let mut cookies = self.0.cookies.write();
                    cookies.insert(key, value);
                }
            });
    }

    pub fn header_cookie(&self) -> String {
        let cookies = self.0.cookies.read();
        cookies
            .iter()
            .enumerate()
            .map(|(idx, (name, value))| {
                format!(
                    "{}={}{}",
                    name,
                    value,
                    if idx == cookies.len() - 1 { "" } else { ";" }
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end()
            .to_string()
    }

    pub fn cookie_changer(&self, reset_timer: Option<bool>, cleanup: Option<bool>) -> bool {
        let my_state = self.0.clone();
        let reset_timer = reset_timer.unwrap_or(true);
        let cleanup = cleanup.unwrap_or(false);
        let config = self.0.config.read();
        if config.cookie_array.is_empty() {
            *my_state.changing.write() = false;
            false
        } else {
            *my_state.change_flag.write() = 0;
            *my_state.changing.write() = true;
            if !cleanup {
                // rotate the cookie
                let mut index = my_state.current_index.write();
                let array_len = config.cookie_array.len();
                *index = (*index + 1) % array_len;
                println!("{}", "Changing cookie".green());
            }
            // set timeout callback
            let dur = if config.rproxy.is_empty() || config.rproxy == ENDPOINT {
                15000 + my_state.timestamp.read().clone() - chrono::Utc::now().timestamp_millis()
            } else {
                0
            };
            let dur = Duration::from_millis(dur as u64);
            let self_clone = self.clone();
            spawn(timeout(dur, async move {
                spawn(async move { self_clone.on_listen().await });
                if reset_timer {
                    let now = chrono::Utc::now().timestamp_millis();
                    *my_state.timestamp.write() = now;
                }
            }));
            false
        }
    }

    pub fn wait_for_change(&self) -> impl Future<Output = ()> {
        async {
            // if changing is true, wait for it to be false
            let istate = self.0.clone();
            while *istate.changing.read() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    pub fn cookie_cleaner(&self, reason: UselessReason) -> bool {
        let mut config = self.0.config.write();
        if config.current_cookie_info().is_none() {
            return false;
        }
        let current_index = self.0.current_index.read().clone();
        let mut config = self.0.config.write();
        let current_cookie = config.cookie_array.remove(current_index);
        config.cookie.clear();
        config
            .wasted_cookie
            .push(UselessCookie::new(current_cookie.cookie, reason));
        config.save().unwrap_or_else(|e| {
            println!("Failed to save config: {}", e);
        });
        println!("Cleaning Cookie...");
        self.cookie_changer(Some(true), Some(true))
    }

    async fn on_listen_catch(&self) -> Result<bool, ClewdrError> {
        let istate = self.0.clone();
        let config = istate.config.read();
        let percentage = ((*istate.change_times.read() as f32)
            + config.cookie_index.saturating_sub(1) as f32)
            / (istate.total_times as f32)
            * 100.0;
        if !config.cookie.validate() {
            *istate.changing.write() = false;
            print!("{}", "No cookie available, enter apiKey-only mode.".red());
            return Ok(false);
        }
        self.update_cookies(&config.cookie.to_string());
        let rproxy = config.rproxy.clone();
        // drop the lock before the async call
        drop(config);
        let end_point = if rproxy.is_empty() { ENDPOINT } else { &rproxy };
        let end_point = format!("{}/api/bootstrap", end_point);
        let res = SUPER_CLIENT
            .get(end_point.clone())
            .header_append("Origin", ENDPOINT)
            .header_append("Referer", header_ref(""))
            .header_append("Cookie", self.header_cookie())
            .send()
            .await?;
        let res = check_res_err(res).await?;
        let bootstrap = res.json::<Value>().await?;
        if bootstrap["account"].is_null() {
            println!("{}", "Null Error, Useless Cookie".red());
            return Ok(self.cookie_cleaner(UselessReason::Null));
        }
        let memberships = bootstrap["account"]["memberships"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let boot_acc_info = memberships
            .iter()
            .find(|m| {
                m["organization"]["capabilities"]
                    .as_array()
                    .map_or(false, |c| c.iter().any(|c| c.as_str() == Some("chat")))
            })
            .and_then(|m| m["organization"].as_object())
            .ok_or(ClewdrError::UnexpectedNone)?;
        let mut cookie_model = None;
        if let Some(model) = bootstrap.pointer("/statsig/values/layer_configs/HPOHwBLNLQLxkj5Yn4bfSkgCQnBX28kPR7h~1BNKdVLw=/value/console_default_model_override/model")
            .and_then(|m| m.as_str())
        {
            cookie_model = Some(model.to_string());
        }
        if cookie_model.is_none() {
            if let Some(model) = bootstrap.pointer("/statsig/values/dynamic_configs/6zA9wvTedwkzjLxWy9PVe7yydI00XDQ6L5Fejjq~12o8=/value/model")
                .and_then(|m| m.as_str())
            {
                cookie_model = Some(model.to_string());
            }
        }
        let mut is_pro = None;
        if let Some(capabilities) = boot_acc_info["capabilities"].as_array() {
            if capabilities
                .iter()
                .any(|c| c.as_str() == Some("claude_pro"))
            {
                is_pro = Some("claude_pro".to_string());
            } else if capabilities.iter().any(|c| c.as_str() == Some("raven")) {
                is_pro = Some("claude_team_pro".to_string())
            }
        }
        *istate.is_pro.write() = is_pro.clone();
        *istate.cookie_model.write() = cookie_model.clone();

        // Check if cookie model is unknown (not in known models or in config's unknown models)
        let mut config = istate.config.write();
        if let Some(cookie_model) = &cookie_model {
            if !MODELS.contains(&cookie_model.as_str())
                && !config.unknown_models.contains(cookie_model)
            {
                config.unknown_models.push(cookie_model.clone());
                config.save().unwrap_or_else(|e| {
                    println!("Failed to save config: {}", e);
                });
            }
        }

        let model_name = if is_pro.is_some() {
            is_pro.clone().unwrap()
        } else if cookie_model.is_some() {
            cookie_model.clone().unwrap().clone()
        } else {
            String::new()
        };
        if let Some(current_cookie) = config.current_cookie_info() {
            if !model_name.is_empty() {
                current_cookie.model = Some(model_name);
                config.save().unwrap_or_else(|e| {
                    println!("Failed to save config: {}", e);
                });
            }
        }
        if is_pro.is_none()
            && istate.model.read().is_some()
            && istate.model.read().as_ref() != cookie_model.as_ref()
        {
            return Ok(self.cookie_changer(None, None));
        }
        let index = if config.cookie_array.is_empty() {
            "".to_string()
        } else {
            format!("(Index: {}) ", config.cookie_index)
                .blue()
                .to_string()
        };
        let name = boot_acc_info
            .get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| n.split_once("@"))
            .map(|(n, _)| n)
            .unwrap_or_default();
        let email = bootstrap
            .pointer("/account/email_address")
            .and_then(|e| e.as_str())
            .unwrap_or_default();
        let caps = boot_acc_info
            .get("capabilities")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{}Logged in \nname: {}\nmail: {}\ncookieModel: {}\ncapabilities: {}",
            index,
            name.blue(),
            email.blue(),
            cookie_model.unwrap_or_default().blue(),
            caps.blue()
        );
        let uuid = boot_acc_info["uuid"]
            .as_str()
            .ok_or(ClewdrError::UnexpectedNone)?;
        let uuid_included = istate.uuid_org_array.read().clone();
        let uuid_included = boot_acc_info["uuid"].as_str().map_or(false, |uuid| {
            uuid_included.iter().any(|u| u.as_str() == uuid)
        });
        let api_disabled_reason = boot_acc_info.get("api_disabled_reason").js_bool();
        let api_disabled_until = boot_acc_info.get("api_disabled_until").js_bool();
        let completed_verification_at = bootstrap
            .get("account")
            .and_then(|a| a.get("completed_verification_at"))
            .js_bool();
        if (uuid_included && percentage <= 100.0 && !config.cookie_array.is_empty())
            || (api_disabled_reason && !api_disabled_until)
            || !completed_verification_at
        {
            let reason = if api_disabled_reason {
                UselessReason::Disabled
            } else if !completed_verification_at {
                UselessReason::Unverified
            } else {
                UselessReason::Overlap
            };
            println!(
                "{}",
                format!("Cookie is useless, reason: {}", reason.to_string().red())
            );
            return Ok(self.cookie_cleaner(reason));
        } else {
            istate.uuid_org_array.write().push(uuid.to_string());
        }

        // Bootstrap complete
        let rproxy = config.rproxy.clone();
        drop(config);
        let end_point = if rproxy.is_empty() { ENDPOINT } else { &rproxy };
        let end_point = format!("{}/api/organizations", end_point);
        let res = SUPER_CLIENT
            .get(end_point.clone())
            .header_append("Origin", ENDPOINT)
            .header_append("Referer", header_ref(""))
            .header_append("Cookie", self.header_cookie())
            .send()
            .await?;
        self.update_cookie_from_res(&res);
        let res = check_res_err(res).await?;
        let ret_json = res.json::<Value>().await?;
        // print bootstrap to out.json, if it exists, overwrite it
        let acc_info = ret_json
            .as_array()
            .and_then(|a| {
                a.iter().find(|v| {
                    v.get("capabilities")
                        .and_then(|c| c.as_array())
                        .map_or(false, |c| c.iter().any(|c| c.as_str() == Some("chat")))
                })
            })
            .ok_or(ClewdrError::UnexpectedNone)?;

        acc_info.get("uuid").and_then(|u| u.as_str()).map(|u| {
            *istate.uuid_org.write() = u.to_string();
        });
        let active_flags = acc_info
            .get("active_flags")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        if !active_flags.is_empty() {
            let now = chrono::Utc::now();
            let formatted_flags = active_flags.iter().map(|f| {
                let expire = f["expires_at"].as_str().unwrap(); // TODO: handle None
                let expire = chrono::DateTime::parse_from_rfc3339(expire).unwrap();
                let diff = expire.to_utc() - now;
                let r#type = f["type"].as_str().unwrap();
                format!(
                    "{}: expires in {} hours",
                    r#type.red(),
                    diff.num_hours().to_string().red()
                )
            });
            let banned = formatted_flags
                .clone()
                .any(|f| f.contains("consumer_banned"));
            let banned_str = if banned {
                "[BANNED] ".red().to_string()
            } else {
                "".to_string()
            };
            println!("{}{}", banned_str, "Your account has warnings:".red());
            for flag in formatted_flags {
                println!("{}", flag);
            }
            let config = istate.config.read();
            let endpoint = if config.rproxy.is_empty() {
                ENDPOINT
            } else {
                &config.rproxy
            };
            let endpoint = format!("{}/api/organizations/{}", endpoint, istate.uuid_org.read());
            let cookies = self.header_cookie();
            if config.settings.clear_flags && !active_flags.is_empty() {
                // drop the lock before the async call
                drop(config);
                let fut = active_flags
                    .iter()
                    .map_while(|f| f.get("type").and_then(|t| t.as_str()))
                    .filter(|&t| t != "consumer_banned" && t != "consumer_restricted_mode")
                    // .map(|t|())
                    .map(|t| {
                        let t = t.to_string();
                        let endpoint = endpoint.clone();
                        let cookies = cookies.clone();
                        async move {
                            let endpoint =
                                format!("{}/flags/{}/dismiss", endpoint.clone(), t.clone());
                            let Ok(res) = SUPER_CLIENT
                                .post(endpoint.clone())
                                .header_append("Origin", ENDPOINT)
                                .header_append("Referer", header_ref(""))
                                .header_append("Cookie", cookies)
                                .send()
                                .await
                                .inspect_err(|e| {
                                    error!("Failed to connect to {}: {}", endpoint, e);
                                })
                            else {
                                return;
                            };
                            self.update_cookie_from_res(&res);
                            let json = match res.json::<Value>().await {
                                Ok(json) => json,
                                Err(e) => {
                                    error!("Failed to parse response json: {}", e);
                                    return;
                                }
                            };
                            let json_error = json.get("error");
                            let error_message = json_error.and_then(|e| e.get("message"));
                            let error_type = json_error.and_then(|e| e.get("type"));
                            let json_detail = json.get("detail");
                            let message = if json_error.is_some() {
                                error_message
                                    .or(json_detail)
                                    .or(error_type)
                                    .and_then(|m| m.as_str())
                                    .unwrap_or_default()
                                    .red()
                                    .to_string()
                            } else {
                                "OK".green().to_string()
                            };
                            println!("{}: {}", t.blue(), message);
                        }
                    })
                    .collect::<Vec<_>>();
                futures::future::join_all(fut).await;
            }
            if banned {
                println!(
                    "{}",
                    "Your account is banned, please use another account.".red()
                );
                return Ok(self.cookie_cleaner(UselessReason::Banned));
            } else {
                // Restricted
                println!("{}", "Your account is restricted.".red());
                if self.0.config.read().settings.skip_restricted {
                    return Ok(self.cookie_changer(None, None));
                }
            }
        }
        let preview_feature_uses_artifacts = bootstrap
            .pointer("/account/settings/preview_feature_uses_artifacts")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
        if preview_feature_uses_artifacts != self.0.config.read().settings.artifacts {
            let endpoint = self.0.config.read().endpoint();
            let endpoint = format!("{}/api/account", endpoint);
            let cookies = self.header_cookie();
            let mut account_settings = bootstrap
                .pointer("/account/settings")
                .and_then(|a| a.as_object())
                .cloned()
                .unwrap_or_default();
            account_settings.insert(
                "preview_feature_uses_artifacts".to_string(),
                Value::Bool(!preview_feature_uses_artifacts),
            );
            let body = json!({
                "settings": account_settings,
            });
            let res = SUPER_CLIENT
                .post(endpoint.clone())
                .header_append("Origin", ENDPOINT)
                .header_append("Referer", header_ref(""))
                .header_append("Cookie", cookies)
                .json(&body)
                .send()
                .await?;

            self.update_cookie_from_res(&res);
            check_res_err(res).await?;
        }
        *self.0.changing.write() = false;
        let endpoint = self.0.config.read().endpoint();
        let uuid = acc_info
            .get("uuid")
            .and_then(|u| u.as_str())
            .unwrap_or_default();
        let endpoint = format!("{}/api/organizations/{}/chat_conversations", endpoint, uuid);
        let cookies = self.header_cookie();
        // mess the cookie a bit to see error message
        let res = SUPER_CLIENT
            .get(endpoint.clone())
            .header_append("Origin", ENDPOINT)
            .header_append("Referer", header_ref(""))
            .header_append("Cookie", cookies)
            .send()
            .await?;
        self.update_cookie_from_res(&res);
        let ret_json = res.json::<Value>().await?;
        let cons = ret_json.as_array().cloned().unwrap_or_default();
        // TODO: Do I need a pool to delete the conversations?
        let futures = cons
            .iter()
            .filter_map(|c| {
                c.get("uuid")
                    .and_then(|u| u.as_str())
                    .map(|u| u.to_string())
            })
            .map(|u| self.delete_chat(u))
            .collect::<Vec<_>>();
        futures::future::join_all(futures).await;
        Ok(true)
    }

    pub async fn on_listen(&self) -> bool {
        let istate = self.0.clone();
        let mut config = istate.config.write();
        if istate.first_login.read().clone() {
            *istate.first_login.write() = false;
            // get time now
            let now = chrono::Utc::now().timestamp_millis();
            *istate.timestamp.write() = now;
            const TITLE: &str = formatc!(
                "Clewdr v{} by {}",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_AUTHORS")
            );
            println!("{}", TITLE.blue());
            println!("Listening on {}", config.address().green());
            // println!("Config:\n{:?}", config);
            // TODO: Local tunnel
        }
        if !config.cookie_array.is_empty() {
            let current_cookie = config.current_cookie_info().unwrap().clone();
            config.cookie = current_cookie.cookie.clone();

            *istate.change_times.write() += 1;
            if istate.model.read().is_some()
                && current_cookie.model.is_some()
                && !current_cookie.is_pro()
                && istate.model.read().as_ref().unwrap() != &current_cookie.model.unwrap()
            {
                return self.cookie_changer(Some(false), None);
            }
        }
        drop(config);
        let res = self.on_listen_catch().await;
        match res {
            Ok(b) => b,
            Err(ClewdrError::JsError(v)) => {
                if Some(json!("Invalid authorization")) == v.message {
                    error!("{}", "Invalid authorization".red());
                    return self.cookie_cleaner(UselessReason::Invalid);
                } else {
                    false
                }
            }
            Err(e) => {
                error!("CLewdR: {}", e);
                self.cookie_changer(None, None);
                false
            }
        }
    }

    pub async fn delete_chat(&self, uuid: String) -> Result<(), ClewdrError> {
        if uuid.is_empty() {
            return Ok(());
        }
        let istate = self.0.clone();
        let conv_uuid = istate.conv_uuid.read().clone();
        let Some(conv_uuid) = conv_uuid else {
            return Ok(());
        };
        if uuid == conv_uuid {
            istate.conv_uuid.write().take();
            *istate.conv_depth.write() = 0;
        }
        if istate.config.read().settings.preserve_chats {
            return Ok(());
        }
        let endpoint = istate.config.read().endpoint();
        let uuid_org = istate.uuid_org.read().clone();
        let endpoint = format!(
            "{}/api/organizations/{}/chat_conversations/{}",
            endpoint, uuid_org, uuid
        );
        let cookies = self.header_cookie();
        let res = SUPER_CLIENT
            .delete(endpoint.clone())
            .header_append("Origin", ENDPOINT)
            .header_append("Referer", header_ref(""))
            .header_append("Cookie", cookies)
            .send()
            .await
            .inspect_err(|e| {
                error!("Failed to connect to {}: {}", endpoint, e);
            })?;
        self.update_cookie_from_res(&res);
        Ok(())
    }
}

async fn api_options() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert(
        "Access-Control-Allow-Headers",
        "Authorization, Content-Type".parse().unwrap(),
    );
    headers.insert(
        "Access-Control-Allow-Methods",
        "POST, GET, OPTIONS".parse().unwrap(),
    );
    headers
}

async fn get_models(
    headers: HeaderMap,
    State(api_state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let api_state = api_state.0;
    let authorization = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    // TODO: get api_rproxy from url query
    let api_rproxy = api_state.config.read().api_rproxy.clone();
    let models = if authorization.matches("oaiKey:").count() > 0 && !api_rproxy.is_empty() {
        let url = format!("{}/v1/models", api_rproxy);
        let key = authorization.replace("oaiKey:", ",");
        if let Some((key, _)) = key.split_once(",") {
            let key = key.trim();
            let resp = NORMAL_CLIENT
                .get(&url)
                .header("Authorization", key)
                .send()
                .await
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let models = resp
                .json::<Value>()
                .await
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            models["data"].as_array().cloned().unwrap_or_default()
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    let config = api_state.config.read();
    let mut data = MODELS
        .iter()
        .cloned()
        .chain(config.unknown_models.iter().map(|s| s.as_str()))
        .map(|name| {
            json!({
                "id":name,
            })
        })
        .chain(models)
        .collect::<Vec<_>>();
    data.sort_unstable_by(|a, b| {
        let a = a["id"].as_str().unwrap_or("");
        let b = b["id"].as_str().unwrap_or("");
        a.cmp(b)
    });
    data.dedup();
    // write to model_list
    let mut model_list = api_state.model_list.write();
    model_list.clear();
    model_list.extend(
        data.iter()
            .filter_map(|model| model["id"].as_str().map(|s| s.to_string())),
    );
    let response = json!({
        "data": data,
    });
    Ok(Json(response))
}

async fn api_complete() -> (StatusCode, Json<Value>) {
    let json = json!(
        {
            "error":{
                "message":                "Clewdr: Set \"Chat Completion source\" to OpenAI instead of Claude. Enable \"External\" models aswell",
                "code": 404,
            }
        }
    );
    (StatusCode::NOT_FOUND, Json(json))
}

async fn api_fallback(req: Request) -> Html<&'static str> {
    let url = req.uri().path();
    if !["/", "/v1", "/favicon.ico"].contains(&url) {
        println!("Unknown request url: {}", url);
    }
    const VX_BY_AUTHOR: &str = formatc!(
        "v{} by {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_AUTHORS")
    );
    Html(concatc!(
        r#"
<html><head>
<meta charset="utf-8">
<script>
function copyToClipboard(text) {
  var textarea = document.createElement("textarea");
  textarea.textContent = text;
  textarea.style.position = "fixed";
  document.body.appendChild(textarea);
  textarea.select();
  try {
    return document.execCommand("copy");
  } catch (ex) {
    console.warn("Copy to clipboard failed.", ex);
    return false;
  } finally {
    document.body.removeChild(textarea);
  }
}
function copyLink(event) {
  event.preventDefault();
  const url = new URL(window.location.href);
  const link = url.protocol + '//' + url.host + '/v1';
  copyToClipboard(link);
  alert('链接已复制: ' + link);
}
</script>
<style id="VMst0.014418824593286361">rt.katakana-terminator-rt::before { content: attr(data-rt); }</style><script id="simplify-jobs-page-script" src="chrome-extension://pbanhockgagggenencehbnadejlgchfc/js/pageScript.bundle.js"></script></head>
<body>
Clewdr "#,
        VX_BY_AUTHOR,
        r#"<br><br>完全开源、免费且禁止商用<br><br>点击复制反向代理: <a href="v1" onclick="copyLink(event)">Copy Link</a><br>填入OpenAI API反向代理并选择OpenAI分类中的claude模型（酒馆需打开Show "External" models，仅在api模式有模型选择差异）<br><br>教程与FAQ: <a href="https://rentry.org/teralomaniac_clewd" target="FAQ">Rentry</a> | <a href="https://discord.com/invite/B7Wr25Z7BZ" target="FAQ">Discord</a><br><br><br>❗警惕任何高风险cookie/伪api(25k cookie)购买服务，以及破坏中文AI开源共享环境倒卖免费资源抹去署名的群组（🈲黑名单：酒馆小二、AI新服务、浅睡(鲑鱼)、赛博女友制作人(青麈/overloaded/科普晓百生)🈲）</body></html>"#
    ))
}
