use anyhow::{Error, Result};
use chrono::format;
use figlet_rs::FIGfont;
use rquest::Response;
use serde_json::{Number, Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};
use tracing::error;

const R: [(&str, &str); 5] = [
    ("user", "Human"),
    ("assistant", "Assistant"),
    ("system", ""),
    ("example_user", "H"),
    ("example_assistant", "A"),
];

static REPLACEMENT: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| HashMap::from(R));
pub static DANGER_CHARS: LazyLock<Vec<char>> = LazyLock::new(|| {
    REPLACEMENT
        .iter()
        .map(|(_, v)| v.chars())
        .flatten()
        .chain(['\n', ':', '\\'])
        .collect()
});

pub fn index_of_any(text: &str, last: Option<bool>) -> i32 {
    let mut indices = vec![index_of_h(text, last), index_of_a(text, last)]
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

pub fn print_out_json(json: &Value) {
    let string = serde_json::to_string_pretty(json).unwrap_or_default();
    let mut file = std::fs::File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open("out.json")
        .unwrap();
    std::io::Write::write_all(&mut file, string.as_bytes()).unwrap();
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
    Json(#[from] serde_json::Error),
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("Rquest error: {0}")]
    Rquest(#[from] rquest::Error),
    #[error("UTF8 error: {0}")]
    UTF8(#[from] std::string::FromUtf8Error),
}

pub const ENDPOINT: &str = "https://api.claude.ai";

pub trait InvalidAuth {
    fn invalid_auth(self) -> Result<Self, ClewdrError>
    where
        Self: Sized;
}

impl InvalidAuth for Option<Value> {
    fn invalid_auth(self) -> Result<Option<Value>, ClewdrError> {
        if let Some(json) = self {
            invalid_auth(json).map(Some)
        } else {
            Ok(self)
        }
    }
}

pub fn invalid_auth(json: Value) -> Result<Value, ClewdrError> {
    if let Some(err) = json.get("error") {
        if let Some(msg) = err.get("message") {
            if msg
                .as_str()
                .map_or(false, |m| m.contains("Invalid authorization"))
            {
                return Err(ClewdrError::InvalidAuth);
            }
        }
    }
    Ok(json)
}

pub fn header_ref(ref_path: &str) -> String {
    if ref_path.is_empty() {
        format!("{}/", ENDPOINT)
    } else {
        format!("{}/chat/{}", ENDPOINT, ref_path)
    }
}

pub async fn check_res_err(res: Response, ret_json: &mut Option<Value>) -> Result<Value> {
    let mut ret = json!({
        "name": "Error",
    });
    let status = res.status();
    if !status.is_success() {
        ret["message"] = json!(format!("Unexpected response code: {}", status));
        error!("Unexpected response code: {}", status);
    }
    let text = res.text().await.unwrap_or_default();
    let json = serde_json::from_str::<Value>(&text).inspect_err(|e| {
        error!("Failed to parse response: {}\n{}", e, text);
    })?;
    if !json.is_null() {
        *ret_json = Some(json.clone());
    }
    let Some(err_api) = json.get("error") else {
        return Ok(ret);
    };
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
    Err(Error::msg(ret))
}
