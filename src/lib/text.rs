use regex::RegexBuilder;

use crate::{
    api::AppState,
    completion::{Message, RetryStrategy},
    utils::REPLACEMENT,
};

impl AppState {
    pub fn handle_messages(
        &self,
        messages: &[Message],
        strategy: RetryStrategy,
    ) -> (String, Vec<String>) {
        let re_scenario =
            RegexBuilder::new(r"^\[Circumstances and context of the dialogue: ([\s\S]+?)\.?\]$")
                .case_insensitive(true)
                .build()
                .unwrap();
        let re_personality = RegexBuilder::new(r"^\[([\s\S]+?)'s personality: ([\s\S]+?)\]$")
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
                .and_then(|c| c.get(1))
                .map(|c| c.as_str())
            {
                let re = RegexBuilder::new(r"{{scenario}}")
                    .multi_line(true)
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                m.content = re
                    .replace_all(&s.config.read().scenario_format, scenario)
                    .to_string();
                m.scenario = Some(true);
            }
            let personalities = re_personality.captures(&m.content);
            if personalities.is_some() && personalities.as_ref().unwrap().len() == 3 {
                let re1 = RegexBuilder::new(r"{{char}}")
                    .multi_line(true)
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                let re2 = RegexBuilder::new(r"{{personality}}")
                    .multi_line(true)
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
}
