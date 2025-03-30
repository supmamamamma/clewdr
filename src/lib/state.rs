use parking_lot::RwLock;
use regex::Regex;
use regex::RegexBuilder;
use rquest::Response;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::{collections::HashMap, sync::Arc};
use tokio::time::sleep;
use tokio::{spawn, time::Duration};
use tracing::debug;
use tracing::error;
use tracing::warn;

use crate::client::AppendHeaders;
use crate::client::SUPER_CLIENT;
use crate::config::UselessReason;
use crate::error::ClewdrError;
use crate::{config::Config, utils::ENDPOINT};

#[derive(Default)]
pub struct InnerState {
    pub config: RwLock<Config>,
    init_length: u64,
    rotating: AtomicBool,
    pub is_pro: RwLock<Option<String>>,
    pub uuid_org: RwLock<String>,
    pub model: RwLock<Option<String>>,
    cookies: RwLock<HashMap<String, String>>,
    pub uuid_org_array: RwLock<Vec<String>>,
    pub conv_uuid: RwLock<Option<String>>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<InnerState>);

impl AppState {
    pub fn new(config: Config) -> Self {
        let m = InnerState {
            init_length: config.cookie_array_len() as u64,
            config: RwLock::new(config),
            ..Default::default()
        };
        let m = Arc::new(m);
        AppState(m)
    }

    pub fn update_cookie_from_res(&self, res: &Response) {
        if let Some(s) = res
            .headers()
            .get("set-cookie")
            .and_then(|h| h.to_str().ok())
        {
            self.update_cookies(s)
        }
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

    pub fn header_cookie(&self) -> Result<String, ClewdrError> {
        if self.0.rotating.load(Ordering::Relaxed) {
            return Err(ClewdrError::CookieRotating);
        }
        let cookies = self.0.cookies.read();
        Ok(cookies
            .iter()
            .map(|(name, value)| format!("{}={}", name, value))
            .collect::<Vec<_>>()
            .join("; ")
            .trim()
            .to_string())
    }

    pub fn cookie_rotate(&self, reason: UselessReason) {
        let self_clone = self.clone();
        spawn(async move {
            if let Ok(err) = self_clone.delete_chat().await {
                error!("Failed to delete chat: {:?}", err);
            }
        });
        static SHIFTS: AtomicU64 = AtomicU64::new(0);
        if SHIFTS.load(Ordering::Relaxed) == self.0.init_length {
            error!("Cookie used up, not rotating");
            return;
        }
        let mut config = self.0.config.write();
        let Some(current_cookie) = config.current_cookie_info() else {
            return;
        };
        match reason {
            UselessReason::Temporary(i) => {
                warn!("Temporary useless cookie, not cleaning");
                current_cookie.reset_time = Some(i);
                config.save().unwrap_or_else(|e| {
                    error!("Failed to save config: {}", e);
                });
            }
            _ => {
                // if reason is not temporary, clean cookie
                config.cookie_cleaner(reason);
            }
        }
        // rotate the cookie
        config.rotate_cookie();
        config.save().unwrap_or_else(|e| {
            error!("Failed to save config: {}", e);
        });
        // set timeout callback
        let dur = if config.rproxy.is_empty() || config.rproxy == ENDPOINT {
            warn!("Waiting 15 seconds to change cookie");
            15
        } else {
            0
        };
        let dur = Duration::from_secs(dur as u64);
        let self_clone = self.clone();
        SHIFTS.fetch_add(1, Ordering::Relaxed);
        spawn(async move {
            self_clone.0.rotating.store(true, Ordering::Relaxed);
            sleep(dur).await;
            warn!("Cookie rotating complete");
            self_clone.0.rotating.store(false, Ordering::Relaxed);
            self_clone.bootstrap().await;
        });
    }

    pub async fn delete_chat(&self) -> Result<(), ClewdrError> {
        let uuid = self.0.conv_uuid.write().take();
        let config = self.0.config.read().clone();
        let uuid_org = self.0.uuid_org.read().clone();
        if uuid.clone().is_none_or(|u| u.is_empty()) {
            return Ok(());
        }
        let uuid = uuid.unwrap();
        if config.settings.preserve_chats {
            return Ok(());
        }
        debug!("Deleting chat: {}", uuid);
        let endpoint = config.endpoint("api/organizations");
        let endpoint = format!("{}/{}/chat_conversations/{}", endpoint, uuid_org, uuid);
        let res = SUPER_CLIENT
            .delete(endpoint.clone())
            .append_headers("", self.header_cookie()?)
            .send()
            .await?;
        self.update_cookie_from_res(&res);
        Ok(())
    }
}
