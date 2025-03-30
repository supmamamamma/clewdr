use colored::Colorize;
use serde_json::{Value, json};
use tracing::{error, warn};

use crate::{
    client::{AppendHeaders, SUPER_CLIENT},
    config::UselessReason,
    error::{ClewdrError, check_res_err},
    state::AppState,
    utils::{ENDPOINT, JsBool, MODELS},
};

impl AppState {
    pub async fn bootstrap(&self) {
        let istate = self.0.clone();
        {
            let mut config = istate.config.write();
            if let Some(current_cookie) = config.current_cookie_info().cloned() {
                config.cookie = current_cookie.cookie.clone();
                if istate.model.read().is_some()
                    && current_cookie.model.is_some()
                    && !current_cookie.is_pro()
                    && istate.model.read().as_ref().unwrap() != &current_cookie.model.unwrap()
                {
                    self.cookie_rotate(UselessReason::Null);
                    return;
                }
            }
        }

        let res = self.try_bootstrap().await;
        if let Err(ClewdrError::JsError(v)) = res {
            if Some(json!("Invalid authorization")) == v.message {
                error!("{}", "Invalid authorization".red());
                self.cookie_rotate(UselessReason::Invalid);
            }
        }
    }

    async fn try_bootstrap(&self) -> Result<(), ClewdrError> {
        let istate = self.0.clone();
        let config = istate.config.read().clone();
        if !config.cookie.validate() {
            error!("{}", "Invalid Cookie, enter apiKey-only mode.".red());
            return Err(ClewdrError::InvalidAuth);
        }
        self.update_cookies(&config.cookie.to_string());
        let end_point = config.endpoint("api/bootstrap");
        let res = SUPER_CLIENT
            .get(end_point.clone())
            .append_headers("", self.header_cookie()?)
            .send()
            .await?;
        let res = check_res_err(res).await?;
        let bootstrap = res.json::<Value>().await?;
        if bootstrap["account"].is_null() {
            println!("{}", "Null Error, Useless Cookie".red());
            self.cookie_rotate(UselessReason::Null);
            return Err(ClewdrError::InvalidAuth);
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
                    .is_some_and(|c| c.iter().any(|c| c.as_str() == Some("chat")))
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
        {
            // drop lock by using a new scope
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

            let model_name = is_pro.clone().or(cookie_model.clone()).unwrap_or_default();
            if let Some(current_cookie) = config.current_cookie_info() {
                if !model_name.is_empty() {
                    current_cookie.model = Some(model_name);
                    config.save().unwrap_or_else(|e| {
                        println!("Failed to save config: {}", e);
                    });
                }
            }
        }
        if is_pro.is_none()
            && istate.model.read().is_some()
            && istate.model.read().as_ref() != cookie_model.as_ref()
        {
            self.cookie_rotate(UselessReason::Null);
            return Err(ClewdrError::InvalidAuth);
        }
        let config = istate.config.read().clone();
        let index = if config.index() < 0 {
            "".to_string()
        } else {
            format!("(Index: {}) ", config.index()).blue().to_string()
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
        let uuid_included = boot_acc_info["uuid"]
            .as_str()
            .is_some_and(|uuid| uuid_included.iter().any(|u| u.as_str() == uuid));
        let api_disabled_reason = boot_acc_info.get("api_disabled_reason").js_bool();
        let api_disabled_until = boot_acc_info.get("api_disabled_until").js_bool();
        let completed_verification_at = bootstrap
            .get("account")
            .and_then(|a| a.get("completed_verification_at"))
            .js_bool();
        if (uuid_included && !config.cookie_array_len() == 0)
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
            println!("Cookie is useless, reason: {}", reason.to_string().red());
            self.cookie_rotate(reason);
            return Err(ClewdrError::InvalidAuth);
        } else {
            istate.uuid_org_array.write().push(uuid.to_string());
        }

        // Bootstrap complete
        let rproxy = config.rproxy.clone();
        let end_point = if rproxy.is_empty() { ENDPOINT } else { &rproxy };
        let end_point = format!("{}/api/organizations", end_point);
        let res = SUPER_CLIENT
            .get(end_point.clone())
            .append_headers("", self.header_cookie()?)
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
                        .is_some_and(|c| c.iter().any(|c| c.as_str() == Some("chat")))
                })
            })
            .ok_or(ClewdrError::UnexpectedNone)?;

        if let Some(u) = acc_info.get("uuid").and_then(|u| u.as_str()) {
            *istate.uuid_org.write() = u.to_string();
        }
        let active_flags = acc_info
            .get("active_flags")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        if !active_flags.is_empty() {
            let now = chrono::Utc::now();
            let mut restrict_until = 0;
            let formatted_flags = active_flags
                .iter()
                .map_while(|f| {
                    let expire = f["expires_at"].as_str()?;
                    let expire = chrono::DateTime::parse_from_rfc3339(expire).ok()?;
                    let timestamp = expire.timestamp();
                    restrict_until = timestamp.max(restrict_until);
                    let diff = expire.to_utc() - now;
                    let r#type = f["type"].as_str()?;
                    Some(format!(
                        "{}: expires in {} hours",
                        r#type.red(),
                        diff.num_hours().to_string().red()
                    ))
                })
                .collect::<Vec<_>>();
            let banned = active_flags
                .iter()
                .any(|f| f["type"].as_str() == Some("consumer_banned"));
            let banned_str = if banned {
                "[BANNED] ".red().to_string()
            } else {
                "".to_string()
            };
            println!("{}{}", banned_str, "Your account has warnings:".red());
            for flag in formatted_flags {
                println!("{}", flag);
            }
            if banned {
                println!(
                    "{}",
                    "Your account is banned, please use another account.".red()
                );
                self.cookie_rotate(UselessReason::Banned);
                return Err(ClewdrError::InvalidAuth);
            } else {
                // Restricted
                println!("{}", "Your account is restricted.".red());
                if self.0.config.read().settings.skip_restricted && restrict_until > 0 {
                    warn!("skip_restricted is enabled, skipping...");
                    self.cookie_rotate(UselessReason::Temporary(restrict_until));
                    return Ok(());
                }
            }
        }
        let preview_feature_uses_artifacts = bootstrap
            .pointer("/account/settings/preview_feature_uses_artifacts")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
        if preview_feature_uses_artifacts != self.0.config.read().settings.artifacts {
            let endpoint = self.0.config.read().endpoint("api/account");
            let endpoint = format!("{}/api/account", endpoint);
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
                .append_headers("", self.header_cookie()?)
                .json(&body)
                .send()
                .await?;

            self.update_cookie_from_res(&res);
            check_res_err(res).await?;
        }
        Ok(())
    }
}
