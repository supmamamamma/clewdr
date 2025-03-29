use colored::Colorize;
use rand::{Rng, rng};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Display};
use tracing::{info, warn};

use crate::{error::ClewdrError, utils::ENDPOINT};

pub const CONFIG_NAME: &str = "config.toml";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum UselessReason {
    Null,
    Disabled,
    Unverified,
    Overlap,
    Banned,
    Invalid,
    Temporary(i64),
}

impl Display for UselessReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UselessReason::Null => write!(f, "Null"),
            UselessReason::Disabled => write!(f, "Disabled"),
            UselessReason::Unverified => write!(f, "Unverified"),
            UselessReason::Overlap => write!(f, "Overlap"),
            UselessReason::Banned => write!(f, "Banned"),
            UselessReason::Invalid => write!(f, "Invalid"),
            UselessReason::Temporary(i) => write!(f, "Temporary {}", i),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UselessCookie {
    pub cookie: Cookie,
    pub reason: UselessReason,
}

impl UselessCookie {
    pub fn new(cookie: Cookie, reason: UselessReason) -> Self {
        Self { cookie, reason }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CookieInfo {
    pub cookie: Cookie,
    pub model: Option<String>,
    #[serde(deserialize_with = "validate_reset")]
    #[serde(default)]
    pub reset_time: Option<i64>,
}

fn validate_reset<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Ok(value) = Option::<i64>::deserialize(deserializer) else {
        return Ok(None);
    };
    if let Some(v) = value {
        let Some(time) = chrono::DateTime::from_timestamp(v, 0) else {
            warn!("Invalid reset time: {}", v);
            return Ok(None);
        };
        let now = chrono::Utc::now();
        if time < now {
            info!("Cookie reset time is in the past: {}", time);
            return Ok(None);
        }
        let remaining_time = time - now;
        info!("Cookie reset in {} hours", remaining_time.num_hours());
    }
    Ok(value)
}

impl CookieInfo {
    pub fn new(cookie: &str, model: Option<&str>, reset_time: Option<i64>) -> Self {
        Self {
            cookie: Cookie::from(cookie),
            model: model.map(|m| m.to_string()),
            reset_time,
        }
    }
    pub fn is_pro(&self) -> bool {
        self.model
            .as_ref()
            .is_some_and(|model| model.contains("claude") && model.contains("_pro"))
    }
}

#[derive(Clone)]
pub struct Cookie {
    inner: String,
}

impl Cookie {
    pub fn validate(&self) -> bool {
        // Check if the cookie is valid
        let re = regex::Regex::new(r"sk-ant-sid01-[0-9A-Za-z_-]{86}-[0-9A-Za-z_-]{6}AA").unwrap();
        re.is_match(&self.inner)
    }

    pub fn clear(&mut self) {
        // Clear the cookie
        self.inner.clear();
    }
}

impl From<&str> for Cookie {
    fn from(cookie: &str) -> Self {
        // only keep '=' '_' '-' and alphanumeric characters
        let cookie = cookie
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '=' || *c == '_' || *c == '-')
            .collect::<String>()
            .trim_start_matches("sessionKey=")
            .to_string();
        let cookie = Self { inner: cookie };
        if !cookie.validate() {
            warn!("Invalid cookie format: {}", cookie);
        }
        cookie
    }
}

impl Display for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sessionKey={}", self.inner)
    }
}

impl Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sessionKey={}", self.inner)
    }
}

impl Serialize for Cookie {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let str = self.to_string();
        serializer.serialize_str(&str)
    }
}

impl<'de> Deserialize<'de> for Cookie {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Cookie::from(s.as_str()))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    // Cookie configurations
    pub cookie: Cookie,
    cookie_array: Vec<CookieInfo>,
    pub wasted_cookie: Vec<UselessCookie>,
    pub unknown_models: Vec<String>,

    // Network settings
    pub cookie_counter: u32,
    cookie_index: i32,
    pub proxy_password: String,
    ip: String,
    port: u16,
    pub local_tunnel: bool,

    // Performance settings
    pub buffer_size: u32,
    pub system_interval: u32,

    // Proxy configurations
    pub rproxy: String,
    pub api_rproxy: String,

    // Token handling
    pub placeholder_token: String,
    pub placeholder_byte: String,

    // Prompt templates
    pub prompt_experiment_first: String,
    pub prompt_experiment_next: String,
    pub personality_format: String,
    pub scenario_format: String,

    // Nested settings section
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    pub renew_always: bool,
    pub retry_regenerate: bool,
    pub prompt_experiments: bool,
    pub system_experiments: bool,
    pub prevent_imperson: bool,
    pub all_samples: bool,
    pub no_samples: bool,
    pub strip_assistant: bool,
    pub strip_human: bool,
    pub pass_params: bool,
    pub clear_flags: bool,
    pub preserve_chats: bool,
    pub log_messages: bool,
    pub full_colon: bool,
    pub padtxt: String,
    pub xml_plot: bool,
    pub skip_restricted: bool,
    pub artifacts: bool,
}

const PLACEHOLDER_COOKIE: &str = "sk-ant-sid01----------------------------SET_YOUR_COOKIE_HERE----------------------------------------AAAAAAAA";

impl Default for Config {
    fn default() -> Self {
        Self {
            cookie: Cookie::from(PLACEHOLDER_COOKIE),
            cookie_array: vec![
                CookieInfo::new(PLACEHOLDER_COOKIE, None, None),
                CookieInfo::new(PLACEHOLDER_COOKIE, Some("claude_pro"), None),
            ],
            wasted_cookie: Vec::new(),
            unknown_models: Vec::new(),
            cookie_counter: 3,
            cookie_index: -1,
            proxy_password: String::new(),
            ip: "127.0.0.1".to_string(),
            port: 8484,
            local_tunnel: false,
            buffer_size: 1,
            system_interval: 3,
            rproxy: String::new(),
            api_rproxy: String::new(),
            placeholder_token: String::new(),
            placeholder_byte: String::new(),
            prompt_experiment_first: String::new(),
            prompt_experiment_next: String::new(),
            personality_format: "{{char}}'s personality: {{personality}}".to_string(),
            scenario_format: "Dialogue scenario: {{scenario}}".to_string(),
            settings: Settings::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            renew_always: true,
            retry_regenerate: false,
            prompt_experiments: true,
            system_experiments: true,
            prevent_imperson: true,
            all_samples: false,
            no_samples: false,
            strip_assistant: false,
            strip_human: false,
            pass_params: false,
            clear_flags: true,
            preserve_chats: false,
            log_messages: true,
            full_colon: true,
            padtxt: "1000,1000,15000".to_string(),
            xml_plot: true,
            skip_restricted: false,
            artifacts: false,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, ClewdrError> {
        let file_string = std::fs::read_to_string(CONFIG_NAME).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                let exec_path = std::env::current_exe()?;
                let config_dir = exec_path.parent().ok_or(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Failed to get parent directory",
                ))?;
                let config_path = config_dir.join(CONFIG_NAME);
                std::fs::read_to_string(config_path)
            } else {
                Err(e)
            }
        });
        match file_string {
            Ok(file_string) => {
                let config: Config = toml::de::from_str(&file_string)?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let exec_path = std::env::current_exe()?;
                let config_dir = exec_path.parent().ok_or(ClewdrError::PathNotFound(
                    "Failed to get parent directory".to_string(),
                ))?;
                let default_config = Config::default();
                default_config.save()?;
                let canonical_path = std::fs::canonicalize(config_dir)?;
                println!(
                    "Default config file created at {}/config.toml",
                    canonical_path.display()
                );
                println!("{}", "SET YOUR COOKIE HERE".green());
                Ok(default_config)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn endpoint(&self, path: &str) -> String {
        let endpoint = if self.rproxy.is_empty() {
            ENDPOINT.to_string()
        } else {
            self.rproxy.clone()
        };
        let path = path
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        format!("{}/{}", endpoint, path)
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }

    pub fn save(&self) -> Result<(), ClewdrError> {
        let exec_path = std::env::current_exe()?;
        let config_dir = exec_path.parent().ok_or(ClewdrError::PathNotFound(
            "Failed to get parent directory".to_string(),
        ))?;
        // add file name to the path
        if !config_dir.exists() {
            std::fs::create_dir_all(config_dir)?;
        }
        // Save the config to a file
        let config_path = config_dir.join(CONFIG_NAME);
        let config_string = toml::ser::to_string(self)?;
        std::fs::write(config_path, config_string)?;
        Ok(())
    }

    pub fn current_cookie_info(&mut self) -> Option<&mut CookieInfo> {
        if self.cookie_index < 0 {
            return None;
        }
        if self.cookie_index < self.cookie_array.len() as i32 {
            Some(&mut self.cookie_array[self.cookie_index as usize])
        } else {
            None
        }
    }

    pub fn index(&self) -> i32 {
        self.cookie_index
    }

    pub fn delete_current_cookie(&mut self) -> Option<CookieInfo> {
        if self.cookie_index < 0 {
            return None;
        }
        if self.cookie_index < self.cookie_array.len() as i32 {
            let index = self.cookie_index as usize;
            let removed = self.cookie_array.remove(index);
            if index == self.cookie_array.len() {
                self.cookie_index -= 1;
            }
            warn!("Removed cookie: {}", removed.cookie.to_string().red());
            return Some(removed);
        }
        None
    }

    pub fn cookie_array_len(&self) -> usize {
        self.cookie_array.len()
    }

    pub fn rotate_cookie(&mut self) {
        if self.cookie_array.is_empty() {
            return;
        }
        let array_len = self.cookie_array.len();
        let index = &mut self.cookie_index;
        *index = (*index + 1) % array_len as i32;
        warn!("Rotating cookie to index {}", index.to_string().green());
    }

    pub fn validate(mut self) -> Self {
        if !self.cookie_array.is_empty() && self.cookie_index >= self.cookie_array.len() as i32 {
            self.cookie_index = rng().random_range(0..self.cookie_array.len() as i32);
        }
        // trim and remove non-ASCII characters from cookie
        self.unknown_models = self
            .unknown_models
            .iter()
            .map(|c| c.trim().to_string())
            .collect();
        self.ip = self.ip.trim().to_string();
        self.rproxy = self.rproxy.trim().to_string();
        self.api_rproxy = self
            .api_rproxy
            .trim()
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_string();
        self.settings.padtxt = self.settings.padtxt.trim().to_string();
        self
    }
}
