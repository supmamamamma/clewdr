use figlet_rs::FIGfont;
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::LazyLock};
use tracing::error;

use crate::{config::CONFIG_NAME, error::ClewdrError};

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
        .flat_map(|(_, v)| v.chars())
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

pub fn cwd_or_exec() -> Result<PathBuf, ClewdrError> {
    let cwd = std::env::current_dir().map_err(|_| ClewdrError::PathNotFound("cwd".to_string()))?;
    let cwd_config = cwd.join(CONFIG_NAME);
    if cwd_config.exists() {
        return Ok(cwd);
    }
    let exec_path =
        std::env::current_exe().map_err(|_| ClewdrError::PathNotFound("exec".to_string()))?;
    let exec_dir = exec_path
        .parent()
        .ok_or_else(|| ClewdrError::PathNotFound("exec dir".to_string()))?
        .to_path_buf();
    let exec_config = exec_dir.join(CONFIG_NAME);
    if exec_config.exists() {
        return Ok(exec_dir);
    }
    Err(ClewdrError::PathNotFound(
        "No config found in cwd or exec dir".to_string(),
    ))
}

pub fn print_out_json(json: &impl serde::ser::Serialize, file_name: &str) {
    let text = serde_json::to_string_pretty(json).unwrap_or_default();
    print_out_text(&text, file_name);
}

pub fn print_out_text(text: &str, file_name: &str) {
    let Ok(dir) = cwd_or_exec() else {
        error!("No config found in cwd or exec dir");
        return;
    };
    let log_dir = dir.join("log");
    if !log_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&log_dir) {
            error!("Failed to create log dir: {}\n", e);
            return;
        }
    }
    let file_name = log_dir.join(file_name);
    let Ok(mut file) = std::fs::File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_name)
    else {
        error!("Failed to open file: {}", file_name.display());
        return;
    };
    if let Err(e) = std::io::Write::write_all(&mut file, text.as_bytes()) {
        error!("Failed to write to file: {}\n", e);
    }
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

pub const ENDPOINT: &str = "https://api.claude.ai";
