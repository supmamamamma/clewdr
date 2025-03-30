use rquest::Response;
use serde_json::{Value, json};
use std::fmt::Display;
use tracing::{error, warn};

#[derive(thiserror::Error, Debug)]
pub enum ClewdrError {
    #[error("Invalid authorization")]
    InvalidAuth,
    #[error("Json error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("TOML Deserialize error: {0}")]
    TomlDeError(#[from] toml::de::Error),
    #[error("TOML Serialize error: {0}")]
    TomlSeError(#[from] toml::ser::Error),
    #[error("Regex error: {0}")]
    RegexError(#[from] regex::Error),
    #[error("Rquest error: {0}")]
    RquestError(#[from] rquest::Error),
    #[error("UTF8 error: {0}")]
    UTF8Error(#[from] std::string::FromUtf8Error),
    #[error("JavaScript error {0}")]
    JsError(JsError),
    #[error("Too many requests: {0}")]
    TooManyRequest(JsError, i64),
    #[error("Unexpected None")]
    UnexpectedNone,
    #[error("No valid key")]
    NoValidKey,
    #[error("Please use OpenAI format")]
    WrongCompletionFormat,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid model name: {0}")]
    InvalidModel(String),
    #[error("Config error: {0}")]
    PathNotFound(String),
    #[error("Invalid timestamp: {0}")]
    TimestampError(i64),
    #[error("Wait for cookie rotation")]
    CookieRotating,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct JsError {
    pub name: String,
    pub message: Option<Value>,
    pub status: Option<Value>,
    pub planned: Option<bool>,
    pub r#type: Option<Value>,
}

impl Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        serde_json::to_string(self)
            .map_err(|_| std::fmt::Error)?
            .fmt(f)
    }
}

pub async fn check_res_err(res: Response) -> Result<Response, ClewdrError> {
    let mut ret = JsError {
        name: "Error".to_string(),
        message: None,
        status: None,
        planned: None,
        r#type: None,
    };
    let status = res.status();
    if !status.is_success() {
        ret.message = Some(format!("Unexpected response code: {}", status).into());
        error!("Unexpected response code: {}", status);
    } else {
        return Ok(res);
    }
    let json = res.text().await.inspect_err(|e| {
        error!("Failed to get response: {}\n", e);
    })?;
    let json = serde_json::from_str::<Value>(&json).inspect_err(|e| {
        error!("Failed to parse response: {}\n", e);
    })?;
    let Some(err_api) = json.get("error") else {
        return Err(ClewdrError::JsError(ret));
    };
    ret.status = json.get("status").cloned();
    ret.planned = true.into();
    if !err_api["message"].is_null() {
        ret.message = err_api.get("message").cloned();
    }
    if !err_api["type"].is_null() {
        ret.r#type = err_api.get("type").cloned();
    }
    if status == 429 {
        if let Some(time) = err_api["message"]
            .as_str()
            .and_then(|m| serde_json::from_str::<Value>(m).ok())
            .and_then(|m| m["resetsAt"].as_i64())
        {
            let reset_time = chrono::DateTime::from_timestamp(time, 0)
                .ok_or(ClewdrError::TimestampError(time))?
                .to_utc();
            let now = chrono::Utc::now();
            let diff = reset_time - now;
            let hours = diff.num_hours();
            let message = format!("Rate limit exceeded, expires in {hours} hours");
            warn!(message);
            ret.message = Some(message.into());
            return Err(ClewdrError::TooManyRequest(ret, time));
        }
    }
    Err(ClewdrError::JsError(ret))
}

pub fn check_json_err(json: &Value) -> Value {
    let err = json.get("error");
    let status = json
        .get("status")
        .and_then(|s| s.as_u64())
        .map(|s| s as u16)
        .unwrap_or(500);
    let err_msg = err.and_then(|e| e.get("message")).and_then(|m| m.as_str());
    let mut ret = json!({
        "name": "Error",
        "message": err_msg.unwrap_or("Unknown error"),
    });
    if let Some(err_api) = err {
        ret["status"] = json["status"].clone();
        ret["planned"] = json!(true);
        if !err_api["message"].is_null() {
            ret["message"] = err_api["message"].clone();
        }
        if !err_api["type"].is_null() {
            ret["type"] = err_api["type"].clone();
        }
        if status == 429 {
            if let Some(time) = err_api["message"]
                .as_str()
                .and_then(|m| serde_json::from_str::<Value>(m).ok())
                .and_then(|m| m["resetsAt"].as_i64())
            {
                let Some(reset_time) = chrono::DateTime::from_timestamp(time, 0) else {
                    error!("Failed to parse timestamp: {}", time);
                    return ret;
                };
                let now = chrono::Utc::now();
                let diff = reset_time - now;
                let hours = diff.num_hours();
                let message = format!("Rate limit exceeded, expires in {hours} hours");
                warn!(message);
                ret["message"] = message.into();
            }
        }
    }
    ret
}
