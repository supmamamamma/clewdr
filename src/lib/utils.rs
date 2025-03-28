use eventsource_stream::EventStreamError;
use figlet_rs::FIGfont;
use rquest::Response;
use serde_json::{Value, json};
use std::{collections::HashMap, fmt::Display, sync::LazyLock};
use tracing::{error, warn};

use crate::{completion::Message, stream::ClewdrTransformer};

const R: [(&str, &str); 5] = [
    ("user", "Human"),
    ("assistant", "Assistant"),
    ("system", ""),
    ("example_user", "H"),
    ("example_assistant", "A"),
];

pub const TIME_ZONE: &str = "America/New_York";

pub static REPLACEMENT: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| HashMap::from(R));
pub static DANGER_CHARS: LazyLock<Vec<char>> = LazyLock::new(|| {
    let mut r: Vec<char> = REPLACEMENT
        .iter()
        .map(|(_, v)| v.chars())
        .flatten()
        .chain(['\n', ':', '\\', 'n'])
        .filter(|&c| c != ' ')
        .collect();
    r.sort();
    r.dedup();
    r
});

pub fn clean_json(json: &str) -> &str {
    // return after "data: "
    let Some(json) = json.split("data: ").nth(1) else {
        return json;
    };
    json
}

pub static TEST_MESSAGE: LazyLock<Message> = LazyLock::new(|| Message {
    role: "user".to_string(),
    content: "Hi".to_string(),
    customname: None,
    name: None,
    strip: None,
    jailbreak: None,
    main: None,
    discard: None,
    merged: None,
    personality: None,
    scenario: None,
});

pub fn index_of_any(text: &str, last: Option<bool>) -> i32 {
    let indices = vec![index_of_h(text, last), index_of_a(text, last)]
        .into_iter()
        .filter(|&idx| idx > -1)
        .collect::<Vec<i32>>();
    let last = last.unwrap_or(false);
    if indices.is_empty() {
        -1
    } else if last {
        *indices.iter().max().unwrap() // Last in sorted order is max
    } else {
        *indices.iter().min().unwrap() // First in sorted order is min
    }
}

fn index_of_h(text: &str, last: Option<bool>) -> i32 {
    let last = last.unwrap_or(false);
    let re = regex::Regex::new(r"(?:(?:\\n)|\r|\n){2}((?:Human|H)[:︓：﹕] ?)").unwrap();
    let matches: Vec<_> = re.find_iter(text).collect();

    if matches.is_empty() {
        -1
    } else if last {
        matches.last().unwrap().start() as i32
    } else {
        matches.first().unwrap().start() as i32
    }
}

fn index_of_a(text: &str, last: Option<bool>) -> i32 {
    let last = last.unwrap_or(false);
    let re = regex::Regex::new(r"(?:(?:\\n)|\r|\n){2}((?:Assistant|A)[:︓：﹕] ?)").unwrap();
    let matches: Vec<_> = re.find_iter(text).collect();

    if matches.is_empty() {
        -1
    } else if last {
        matches.last().unwrap().start() as i32
    } else {
        matches.first().unwrap().start() as i32
    }
}

pub fn generic_fixes(text: &str) -> String {
    let re = regex::Regex::new(r"(\r\n|\r|\\n)").unwrap();
    re.replace_all(text, "\n").to_string()
}

pub fn print_out_json(json: &impl serde::ser::Serialize, file_name: &str) {
    let string = serde_json::to_string_pretty(json).unwrap_or_default();
    let mut file = std::fs::File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(file_name)
        .unwrap();
    std::io::Write::write_all(&mut file, string.as_bytes()).unwrap();
}

pub fn print_out_text(text: &str, file_name: &str) {
    let mut file = std::fs::File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(file_name)
        .unwrap();
    std::io::Write::write_all(&mut file, text.as_bytes()).unwrap();
}

pub trait JsBool {
    fn js_bool(&self) -> bool;
}

impl JsBool for Option<&Value> {
    fn js_bool(&self) -> bool {
        match self {
            Some(v) => v.js_bool(),
            None => false,
        }
    }
}

impl JsBool for Value {
    fn js_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Number(n) => {
                // '-0'/'0'/NaN => false
                // other numbers => true
                if let Some(num) = n.as_f64() {
                    if num == 0.0 || num.is_nan() {
                        return false;
                    }
                }
                true
            }
            Value::Bool(b) => *b,
            Value::String(s) => {
                // empty string => false
                // other strings => true
                if s.is_empty() {
                    return false;
                }
                true
            }
            _ => true,
        }
    }
}

pub static BANNER: LazyLock<String> = LazyLock::new(|| {
    let standard_font = FIGfont::standard().unwrap();
    let figure = standard_font.convert("ClewdR");
    let banner = figure.unwrap().to_string();
    format!(
        "{}\nv{} by {}\n",
        banner,
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_AUTHORS")
    )
});

pub const MODELS: [&str; 10] = [
    "claude-3-7-sonnet-20250219",
    "claude-3-5-sonnet-20240620",
    "claude-3-opus-20240229",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
    "claude-2.1",
    "claude-2.0",
    "claude-1.3",
    "claude-instant-1.2",
    "claude-instant-1.1",
];

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
    #[error("Stream cancelled")]
    StreamCancelled(ClewdrTransformer),
    #[error("Stream internal error, no further information available")]
    StreamInternalError(ClewdrTransformer),
    #[error("Stream end")]
    StreamEndNormal(ClewdrTransformer),
    #[error("JavaScript error {0}")]
    JsError(JsError),
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
    #[error("HardCensor detected")]
    HardCensor(ClewdrTransformer),
    #[error("Impersonation detected")]
    Impersonation(ClewdrTransformer),
    #[error("Empty stream")]
    EmptyStream(ClewdrTransformer),
    #[error("Unknown Stream error: {1}")]
    UnknownStreamError(ClewdrTransformer, String),
    #[error("Input stream error: {0}")]
    EventSourceError(EventStreamError<rquest::Error>),
}

pub const ENDPOINT: &str = "https://api.claude.ai";

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

pub fn header_ref(ref_path: &str) -> String {
    if ref_path.is_empty() {
        format!("{}/", ENDPOINT)
    } else {
        format!("{}/chat/{}", ENDPOINT, ref_path)
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
            let reset_time = chrono::DateTime::from_timestamp(time, 0).unwrap().to_utc();
            let now = chrono::Utc::now();
            let diff = reset_time - now;
            let hours = diff.num_hours();
            ret.message.as_mut().map(|msg| {
                let new_msg = format!(", expires in {hours} hours");
                if let Some(str) = msg.as_str() {
                    *msg = format!("{str}{new_msg}").into();
                } else {
                    *msg = new_msg.into();
                }
            });
            warn!("Rate limit exceeded, expires in {} hours", hours);
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
                .and_then(|m| m["resetAt"].as_str().map(|s| s.to_string()))
            {
                let reset_time = chrono::DateTime::parse_from_rfc3339(&time)
                    .unwrap()
                    .to_utc();
                let now = chrono::Utc::now();
                let diff = reset_time - now;
                let hours = diff.num_hours();
                ret.as_object_mut()
                    .and_then(|obj| obj.get_mut("message"))
                    .map(|msg| {
                        let new_msg = format!(", expires in {hours} hours");
                        if let Some(str) = msg.as_str() {
                            *msg = format!("{str}{new_msg}").into();
                        } else {
                            *msg = new_msg.into();
                        }
                    });
            }
        }
    }
    ret
}
