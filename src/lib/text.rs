use claude_tokenizer::count_tokens;
use rand::{Rng, RngCore};
use regex::Regex;
use std::fmt::Write;
use tracing::error;

use crate::{
    completion::Message,
    state::AppState,
    utils::{REPLACEMENT, print_out_text},
};

impl AppState {
    pub fn handle_messages(&self, messages: &[Message]) -> String {
        let real_logs = messages
            .iter()
            .filter(|m| ["assistant", "user"].contains(&m.role.as_str()))
            .collect::<Vec<_>>();
        let sample_logs = messages
            .iter()
            .filter(|m| m.name.as_ref().map(|n| !n.is_empty()).unwrap_or_default())
            .collect::<Vec<_>>();
        let mut merged_logs = sample_logs
            .iter()
            .chain(real_logs.iter())
            .cloned()
            .cloned()
            .collect::<Vec<_>>();
        for m in &mut merged_logs {
            let name = m.name.as_deref().unwrap_or_default();
            m.customname = Some(
                ["assistant", "user"].contains(&m.role.as_str())
                    && m.name.is_some()
                    && !REPLACEMENT.contains_key(name),
            )
        }
        for i in 1..merged_logs.len() {
            let (prev, next) = merged_logs.split_at_mut(i);
            let prev = prev.last_mut().unwrap();
            let next = next.first_mut().unwrap();
            if prev.name.is_some() && prev.name == next.name {
                write!(prev.content, "\n{}", next.content).unwrap();
                next.merged = Some(true);
            } else if next.role != "system" {
                if next.role == prev.role {
                    write!(prev.content, "\n{}", next.content).unwrap();
                    next.merged = Some(true);
                }
            } else {
                // merge system messages
                write!(prev.content, "\n{}", next.content).unwrap();
                next.merged = Some(true);
            }
        }
        let mut system_messages = messages
            .iter()
            .filter(|m| m.role == "system" && m.name.is_none())
            .cloned()
            .collect::<Vec<_>>();
        let sys_messages_len = system_messages.len();
        for (i, m) in system_messages.iter_mut().enumerate() {
            m.main = if i == 0 { Some(true) } else { Some(false) };
            m.jailbreak = if i == sys_messages_len - 1 {
                Some(true)
            } else {
                Some(false)
            };
            if m.content.trim().is_empty() {
                m.discard = Some(true);
            }
        }
        let prompt = messages
            .iter()
            .map_while(|m| self.generate_prompt(m))
            .collect::<Vec<_>>()
            .join("\n\n"); // TODO: Non xml plot is not
        prompt
    }

    pub fn generate_prompt(&self, messages: &Message) -> Option<String> {
        if messages.merged.unwrap_or_default()
            || messages.discard.unwrap_or_default()
            || messages.content.is_empty()
        {
            return None;
        }
        let prefix = if *messages.customname.as_ref().unwrap_or(&false) {
            messages
                .name
                .clone()
                .map(|n| n.replace("_", "") + ": ")
                .unwrap_or_default()
        } else if messages.role != "system"
            || messages
                .name
                .clone()
                .map(|n| !n.is_empty())
                .unwrap_or_default()
        {
            let replace = messages
                .name
                .clone()
                .and_then(|n| REPLACEMENT.get(n.as_str()))
                .or(REPLACEMENT.get(messages.role.as_str()))
                .cloned()
                .unwrap_or(&messages.role);
            format!("{}: ", replace)
        } else {
            REPLACEMENT
                .get(messages.role.as_str())
                .cloned()
                .unwrap_or(&messages.role)
                .to_string()
        };
        return Some(format!("{}{}", prefix, messages.content.trim()));
    }

    pub fn pad_txt(&self, mut content: String) -> String {
        let Ok(mut tokens) = count_tokens(content.as_str()) else {
            error!("Failed to count tokens");
            return content;
        };
        let pad_txt = self.0.config.read().settings.padtxt.clone();
        let pad_txt = pad_txt.split(",").collect::<Vec<_>>();
        let pad_txt = pad_txt.iter().rev().collect::<Vec<_>>();
        let pad_txt = pad_txt
            .iter()
            .map(|s| s.parse::<usize>().unwrap_or(1000))
            .collect::<Vec<_>>();
        let [max_tokens, extra_tokens, min_tokens, ..] = pad_txt.as_slice() else {
            error!("Failed to parse pad_txt");
            return content;
        };
        let placeholder = {
            let h = if tokens > max_tokens - extra_tokens && min_tokens > &0 {
                self.0.config.read().placeholder_byte.clone()
            } else {
                self.0.config.read().placeholder_token.clone()
            };
            if h.is_empty() {
                // random size
                let mut rng = rand::rng();
                let rand_size = rng.random_range(5..15);
                let mut vec = vec![0; rand_size];
                rand::rng().fill_bytes(&mut vec);
                // to hex
                vec.iter().fold(String::new(), |mut acc, b| {
                    let _ = write!(acc, "{b:02X}");
                    acc
                })
            } else {
                h
            }
        };
        let placeholder_tokens = count_tokens(placeholder.as_str()).unwrap_or_default();
        let re = Regex::new(r"<\|padtxt.*?(\d+)t.*?\|>").unwrap();
        for cs in re
            .captures_iter(content.clone().as_str())
            .map(|c| c.iter().collect::<Vec<_>>())
        {
            let (Some(Some(m1)), Some(Some(m2))) = (cs.get(1), cs.get(2)) else {
                continue;
            };
            let m1 = m1.as_str();
            let m2 = m2.as_str();
            tokens += m1.parse::<usize>().unwrap_or_default();
            content = content.replace(
                m1,
                &placeholder.repeat(m2.parse::<usize>().unwrap_or_default() / placeholder_tokens),
            );
        }
        print_out_text(&content, "2.1.placeholder.txt");
        let re = Regex::new(r"<\|padtxt off.*?\|>").unwrap();
        if re.is_match(content.as_str()) {
            let re = Regex::new(r"\s*<\|padtxt.*?\|>\s*").unwrap();
            return re.replace_all(content.as_str(), "\n\n").to_string();
        }
        let padding = placeholder.repeat(
            (*max_tokens).min(if tokens <= max_tokens - extra_tokens {
                max_tokens - tokens
            } else if min_tokens > &0 {
                *min_tokens
            } else {
                *extra_tokens
            }) / placeholder_tokens,
        );
        let re = Regex::new(r"<\|padtxt.*?\|>").unwrap();
        if re.is_match(content.as_str()) {
            content = re.replace(content.as_str(), padding).to_string();
            let re2 = Regex::new(r"\s*<\|padtxt.*?\|>\s*").unwrap();
            content = re2.replace_all(content.as_str(), "\n\n").to_string();
        } else {
            content = format!("{}\n\n\n{}", padding, content.trim());
        }
        content
    }
}
