use std::borrow::Cow;

use claude_tokenizer::count_tokens;
use colored::Colorize;
use rand::{Rng, RngCore};
use serde_json::Value;
use tracing::{error, warn};

use crate::{
    api::AppState,
    completion::{Message, RetryStrategy},
    utils::{REPLACEMENT, print_out_text},
};

impl AppState {
    pub fn handle_messages(
        &self,
        messages: &[Message],
        strategy: RetryStrategy,
    ) -> (String, Vec<String>) {
        let re_scenario = fancy_regex::RegexBuilder::new(
            r"^\[Circumstances and context of the dialogue: ([\s\S]+?)\.?\]$",
        )
        .case_insensitive(true)
        .build()
        .unwrap();
        let re_personality =
            fancy_regex::RegexBuilder::new(r"^\[([\s\S]+?)'s personality: ([\s\S]+?)\]$")
                .case_insensitive(true)
                .build()
                .unwrap();
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
            let name = m.name.as_ref().map(|n| n.as_str()).unwrap_or_default();
            m.customname = Some(
                ["assistant", "user"].contains(&m.role.as_str())
                    && m.name.is_some()
                    && !REPLACEMENT.contains_key(name),
            )
        }
        let s = self.0.as_ref();
        if !s.config.read().settings.xml_plot {
            // TODO: Non-xml plot
            // for (prev, next) in merged_logs.iter().zip(merged_logs.iter().skip(1)) {}
        }
        let mut last_assistant = real_logs
            .iter()
            .rfind(|m| m.role == "assistant" && m.merged.unwrap_or_default())
            .cloned()
            .cloned();
        if s.config.read().settings.strip_assistant {
            last_assistant.as_mut().map(|m| {
                m.strip = Some(true);
            });
        }
        let mut last_user = real_logs
            .iter()
            .rfind(|m| m.role == "user" && m.merged.unwrap_or_default())
            .cloned()
            .cloned();
        if s.config.read().settings.strip_human {
            last_user.as_mut().map(|m| {
                m.strip = Some(true);
            });
        }
        let mut system_messages = messages
            .iter()
            .filter(|m| m.role == "system" && m.name.is_none())
            .cloned()
            .collect::<Vec<_>>();
        let sys_messages_len = system_messages.len();
        for (i, m) in system_messages.iter_mut().enumerate() {
            if let Some(scenario) = re_scenario
                .captures(&m.content)
                .ok()
                .and_then(|c| c)
                .and_then(|c| c.get(1))
                .map(|c| c.as_str())
            {
                let re = fancy_regex::RegexBuilder::new(r"(?m){{scenario}}")
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                m.content = re
                    .replace_all(&s.config.read().scenario_format, scenario)
                    .to_string();
                m.scenario = Some(true);
            }
            let personalities = re_personality.captures(&m.content).ok().and_then(|c| c);
            if personalities.is_some() && personalities.as_ref().unwrap().len() == 3 {
                let re1 = fancy_regex::RegexBuilder::new(r"(?m){{char}}")
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                let re2 = fancy_regex::RegexBuilder::new(r"(?m){{personality}}")
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                let new_content = re1
                    .replace_all(
                        &s.config.read().personality_format,
                        &personalities.as_ref().unwrap()[1],
                    )
                    .to_string();
                m.content = re2
                    .replace_all(&new_content, &personalities.unwrap()[2])
                    .to_string();
                m.personality = Some(true);
            }
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
        // TODO: All sample
        // TODO: Non sample
        let systems: Vec<String> = Vec::new();
        if strategy.is_current() {
            // TODO: current chat
        }
        let prompt = messages
            .iter()
            .map_while(|m| self.generate_prompt(m))
            .collect::<Vec<_>>()
            .join("\n\n"); // TODO: Non xml plot is not
        (prompt, systems)
    }

    pub fn generate_prompt(&self, messages: &Message) -> Option<String> {
        if messages.merged.unwrap_or_default()
            || messages.discard.unwrap_or_default()
            || messages.content.is_empty()
        {
            return None;
        }
        let s = self.0.as_ref();
        if s.config.read().settings.xml_plot {
            let prefix = if *messages.customname.as_ref().unwrap_or(&false) {
                messages.role.clone()
                    + ": "
                    + messages
                        .name
                        .clone()
                        .unwrap_or_default()
                        .replace("_", " ")
                        .as_str()
                    + ": "
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
                let replace = REPLACEMENT
                    .get(messages.role.as_str())
                    .cloned()
                    .unwrap_or(&messages.role)
                    .to_string();
                format!("xmlPlot: {}", replace)
            };
            return Some(format!(
                "{}{}",
                if messages.strip.unwrap_or_default() {
                    String::new()
                } else {
                    prefix
                },
                messages.content
            ));
        } else {
            // TODO: Non-xml plot
        }
        None
    }

    pub fn xml_plot(&self, content: String, non_sys: Option<bool>) -> String {
        let non_sys = non_sys.unwrap_or_default();
        self.0.regex_log.write().clear();
        let content = self.xml_plot_regex(content, 1);
        let merge_tag = MergeTag {
            all: !content.contains("<|Merge Disable|>"),
            system: !content.contains("<|Merge System Disable|>"),
            human: !content.contains("<|Merge Human Disable|>"),
            assistant: !content.contains("<|Merge Assistant Disable|>"),
        };
        let mut content = xml_plot_merge(&content, &merge_tag, non_sys);
        let mut split_content = {
            let re = fancy_regex::Regex::new(r"\n\n(?=Assistant:|Human:)").unwrap();
            re.split(&content)
                .map_while(|s| s.map(|s| s.to_string()).ok())
                .collect::<Vec<_>>()
        };
        let re = fancy_regex::Regex::new(r"(?s)<@(\d+)>(.*?)</@\1>").unwrap();
        while let Some(caps) = re.captures(&content).ok().and_then(|o| o) {
            let index = split_content.len() as isize - caps[1].parse::<isize>().unwrap() - 1;
            if index < 0 {
                warn!("{}", "Invalid index".yellow());
            } else {
                split_content[index as usize] +=
                    &("\n\n".to_string() + caps[2].to_string().as_str());
                content = content.replace(caps[0].to_string().as_str(), "");
            }
        }
        let content = split_content.join("\n\n");
        let re = fancy_regex::Regex::new(r"(?s)<@(\d+)>.*?</@\1>").unwrap();
        let content = re.replace_all(&content, "").to_string();
        let content = self.xml_plot_regex(content, 2);
        let mut content = xml_plot_merge(&content, &merge_tag, non_sys);
        let split_human = content
            .split("\n\nHuman:")
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        // TODO: handle api key
        if split_human.len() > 2
            && split_content
                .last()
                .unwrap()
                .contains("<|Plain Prompt Enable|>")
            && !content.contains("\n\nPlainPrompt:")
        {
            let split = split_human
                .iter()
                .rev()
                .skip(1)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n\nHuman:");
            let re = fancy_regex::Regex::new(r"\n\nHuman: *PlainPrompt:").unwrap();
            content = split.clone()
                + "\n\nPlainPrompt:"
                + re.replace_all(split.as_str(), "\n\nPlainPrompt:")
                    .to_string()
                    .as_str();
        }
        let c = self.xml_plot_regex(content, 3);
        let re1 = fancy_regex::Regex::new(r"(?m)<regex( +order *= *\d)?>.*?</regex>").unwrap();
        let re2 = fancy_regex::Regex::new(r"(?m)\r\n|\r").unwrap();
        let re3 = fancy_regex::Regex::new(r"\s*<\|curtail\|>\s*").unwrap();
        let re4 = fancy_regex::Regex::new(r"\s*<\|join\|>\s*").unwrap();
        let re5 = fancy_regex::Regex::new(r"\s*<\|space\|>\s*").unwrap();
        let re6 = fancy_regex::Regex::new(r"\s*\n\n(H(uman)?|A(ssistant)?): +").unwrap();
        let re7 = fancy_regex::Regex::new(r"<\|(\\.*?)\|>").unwrap();
        let replacer = |caps: &fancy_regex::Captures| {
            let re = regex::Regex::new(r##"\\?""##).unwrap();
            let Some(p1) = caps.get(1).map(|o| o.as_str()) else {
                return caps[0].to_string();
            };
            let p1 = re.replace_all(p1, "\\\"").to_string();
            let Ok(json) = serde_json::from_str::<Value>(p1.as_str()) else {
                return caps[0].to_string();
            };
            return json.to_string();
        };
        let content = re1.replace_all(c.as_str(), "").to_string();
        let content = re2.replace_all(content.as_str(), "\n").to_string();
        let content = re3.replace_all(content.as_str(), "\n").to_string();
        let content = re4.replace_all(content.as_str(), "").to_string();
        let content = re5.replace_all(content.as_str(), " ").to_string();
        let content = re6.replace_all(content.as_str(), "\n\n$1: ").to_string();
        let content = re7.replace_all(content.as_str(), replacer).to_string();
        // TODO: api key logic
        let re1 = fancy_regex::Regex::new(r"\s*<\|(?!padtxt).*?\|>\s*").unwrap();
        let re2 = fancy_regex::Regex::new(r"\s*<\|.*?\|>\s*").unwrap();
        let re3 = fancy_regex::Regex::new(r"^Human: *|\n\nAssistant: *$").unwrap();
        let re4 = fancy_regex::Regex::new(r"(?<=\n)\n(?=\n)").unwrap();
        let c = if !self.0.config.read().settings.padtxt.is_empty() {
            re1.replace_all(content.as_str(), "\n\n").to_string()
        } else {
            re2.replace_all(content.as_str(), "\n\n").to_string()
        }
        .trim()
        .to_string();
        let c = re3.replace_all(c.as_str(), "").to_string();
        re4.replace_all(c.as_str(), "").to_string()
    }

    fn xml_plot_regex(&self, mut content: String, order: i64) -> String {
        let re = fancy_regex::Regex::new(
            format!(
                "(:?)<regex(?: +order *= *{}){}> *\"(/?)(.*)\\1(.*?)\" *: *\"(.*?)\" *</regex>",
                order,
                if order == 2 { "?" } else { "" }
            )
            .as_str(),
        )
        .unwrap();
        let res = re
            .find_iter(content.as_ref())
            .map_while(|m| m.map(|m| m.as_str().to_string()).ok())
            .collect::<Vec<_>>();
        for m in res {
            let re = fancy_regex::Regex::new(
                r##"<regex(?: +order *= *\d)?> *"(/?)(.*)\1(.*?)" *: *"(.*?)" *</regex>"##,
            )
            .unwrap();
            if let Some(caps) = re.captures(m.as_str()).ok().and_then(|o| o) {
                if caps.len() < 5 {
                    warn!("{}", "Regex capture group is less than 5".yellow());
                    continue;
                }
                *self.0.regex_log.write() += "\n";
                let ecma_re = caps[2].to_string();
                let flags = caps[3].to_string();
                let to = caps[4].to_string();
                let Ok(ecma_re) = regress::Regex::with_flags(&ecma_re, flags.as_str()) else {
                    warn!("{}", "ECMA regex is invalid".yellow());
                    continue;
                };
                let to = to.replace("\\?\"", "\\\"')}\"");
                content = ecma_re.replace_all(content.as_ref(), to.as_str()).into();
            }
        }
        content
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
                vec.iter().map(|b| format!("{:02x}", b)).collect::<String>()
            } else {
                h
            }
        };
        let placeholder_tokens = count_tokens(placeholder.as_str()).unwrap_or_default();
        let re = fancy_regex::Regex::new(r"<\|padtxt.*?(\d+)t.*?\|>").unwrap();
        while let [m1, m2, ..] = re
            .find_iter(content.as_str())
            .map_while(|m| m.ok().map(|m| m.as_str().to_string()))
            .collect::<Vec<_>>()
            .as_slice()
        {
            tokens += m1.parse::<usize>().unwrap_or_default();
            content = content.replace(
                m1.as_str(),
                &placeholder.repeat(m2.parse::<usize>().unwrap_or_default() / placeholder_tokens),
            );
        }
        print_out_text(&content, "log/2.1.placeholder.txt");
        let re = fancy_regex::Regex::new(r"<\|padtxt off.*?\|>").unwrap();
        if re.is_match(content.as_str()).unwrap_or_default() {
            let re = fancy_regex::Regex::new(r"\s*<\|padtxt.*?\|>\s*").unwrap();
            return re.replace_all(content.as_str(), "\n\n").to_string();
        }
        let padding = placeholder.repeat(
            max_tokens
                .clone()
                .min(if tokens <= max_tokens - extra_tokens {
                    max_tokens - tokens
                } else if min_tokens > &0 {
                    min_tokens.clone()
                } else {
                    extra_tokens.clone()
                })
                / placeholder_tokens,
        );
        let re = fancy_regex::Regex::new(r"<\|padtxt.*?\|>").unwrap();
        if re.is_match(content.as_str()).unwrap_or_default() {
            content = re.replace(content.as_str(), padding).to_string();
            let re2 = fancy_regex::Regex::new(r"\s*<\|padtxt.*?\|>\s*").unwrap();
            content = re2.replace_all(content.as_str(), "\n\n").to_string();
        } else {
            // TODO: api key logic
            content = format!("{}\n\n\n{}", padding, content.trim());
        }
        content
    }

    pub fn handle_full_colon(
        &self,
        mut prompt: String,
        legacy: bool,
        fusion: bool,
        wedge: String,
    ) -> String {
        if !self.0.config.read().settings.full_colon {
            return prompt;
        }
        if !legacy {
            let re = if fusion {
                fancy_regex::Regex::new(r"(?s)\n(?!\nAssistant:\s*$)(?=\n(Human|Assistant):)")
                    .unwrap()
            } else {
                fancy_regex::Regex::new(r"\n(?=\n(Human|Assistant):)").unwrap()
            };
            prompt = re
                .replace_all(prompt.as_str(), format!("\n{}", wedge))
                .to_string();
        } else {
            let re = if fusion {
                fancy_regex::Regex::new(r"(?s)(?<=\n\nAssistant):(?!\s*$)|(?<=\n\nHuman):").unwrap()
            } else {
                fancy_regex::Regex::new(r"(?<=\n\n(Human|Assistant)):").unwrap()
            };
            prompt = re.replace_all(prompt.as_str(), "﹕").to_string();
        }
        prompt
    }
}

trait Replace<'h> {
    fn replace_all(&self, content: &'h str, to: &str) -> Cow<'h, str>;
}

impl<'h> Replace<'h> for regress::Regex {
    fn replace_all(&self, content: &'h str, to: &str) -> Cow<'h, str> {
        let mut new_content = Cow::from(content);
        // find capture groups in to
        let re = regex::Regex::new(r"\$([1-9]+\d*)").unwrap();
        let to = to.to_string();
        let captures = re
            .find_iter(to.as_str())
            .map_while(|m| m.as_str()[1..].parse::<usize>().ok())
            .collect::<Vec<_>>();
        // find all matches in content
        let mut offset: isize = 0;
        for m in self.find_iter(content) {
            let range_len = m.range().end - m.range().start;
            let range = (m.range().start as isize + offset) as usize
                ..(m.range().end as isize + offset) as usize;
            let grp = m.captures;
            if grp.is_empty() {
                continue;
            }
            let mut to = to.clone();
            for cap in &captures {
                if cap == &0 {
                    continue;
                }
                if let Some(capture) = grp.get(*cap - 1).and_then(|o| o.clone()) {
                    let capture_str = content[capture].to_string();
                    to = to.replace(&format!("${}", cap), &capture_str);
                }
            }
            new_content.to_mut().replace_range(range.clone(), &to);
            offset += to.len() as isize - range_len as isize;
        }
        new_content
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_replace_all() {
        let re = regress::Regex::with_flags(r"(\d+)", "gs").unwrap();
        let content = "123 456 789";
        let to = "$1kkk";
        let result = re.replace_all(content, to);
        assert_eq!(result, "123kkk 456kkk 789kkk");
    }
}

fn xml_plot_merge(content: &str, merge_tag: &MergeTag, non_sys: bool) -> String {
    let re_check = regex::Regex::new(r"(\n\n|^\s*)xmlPlot:\s*").unwrap();
    let mut content = content.to_string();

    if re_check.is_match(&content) {
        if !non_sys {
            let re_remove = regress::Regex::with_flags(
                r"(\n\n|^\s*)(?<!\n\n(Human|Assistant):.*?)xmlPlot:\s*",
                "s",
            )
            .unwrap();
            content = re_remove.replace_all(&content, "$1").to_string();
        }
        let re = regex::Regex::new(r"(\n\n|^\s*)xmlPlot: *").unwrap();
        let to = if merge_tag.system && merge_tag.human && merge_tag.all {
            "\n\nHuman: "
        } else {
            "$1"
        };
        content = re.replace_all(&content, to).to_string();
    }

    if merge_tag.all && merge_tag.human {
        let re =
            fancy_regex::Regex::new(r"(?s)(?:\n\n|^\s*)Human:(.*?(?:\n\nAssistant:|$))").unwrap();
        let replacer = |caps: &fancy_regex::Captures| {
            let re = regex::Regex::new(r"\n\nHuman:\s*").unwrap();
            if caps.len() < 2 {
                return caps[0].to_string();
            }
            let p1 = caps.get(1).unwrap().as_str();
            let p1 = re.replace_all(p1, "\n\n");
            return format!("\n\nHuman:{}", p1);
        };
        content = re.replace_all(&content, replacer).to_string();
    }
    if merge_tag.all && merge_tag.assistant {
        let re = fancy_regex::RegexBuilder::new(r"(?s)\n\nAssistant:(.*?(?:\n\nHuman:|$))")
            .build()
            .unwrap();
        let replacer = |caps: &fancy_regex::Captures| {
            let re = regex::Regex::new(r"\n\nAssistant:\s*").unwrap();
            if caps.len() < 2 {
                return caps[0].to_string();
            }
            let p1 = caps.get(1).unwrap().as_str();
            let p1 = re.replace_all(p1, "\n\n");
            return format!("\n\nAssistant:{}", p1);
        };
        content = re.replace_all(&content, replacer).to_string();
    }
    content
}

struct MergeTag {
    all: bool,
    system: bool,
    human: bool,
    assistant: bool,
}
