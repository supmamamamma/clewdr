use itertools::Itertools;
use serde::Serialize;
use std::fmt::Write;

use crate::{
    types::message::{ContentBlock, ImageSource, Message, MessageContent, Role},
    utils::print_out_text,
};

#[derive(Default, Debug, Serialize)]
pub struct Merged {
    pub head: String,
    pub tail: String,
    #[serde(skip)]
    pub images: Vec<ImageSource>,
}

pub fn merge_messages(msgs: Vec<Message>, user_real_roles: bool) -> Option<Merged> {
    let line_breaks = if user_real_roles { "\n\n\x08" } else { "\n\n" };
    if msgs.is_empty() {
        return None;
    }
    let size = size_of_val(&msgs);
    let mut w = String::with_capacity(size);
    let mut imgs: Vec<ImageSource> = vec![];

    let chunks = msgs
        .into_iter()
        .map_while(|m| match m.content {
            MessageContent::Blocks { content } => {
                let blocks = content
                    .into_iter()
                    .map_while(|b| match b {
                        ContentBlock::Text { text } => Some(text.trim().to_string()),
                        ContentBlock::Image { source } => {
                            imgs.push(source);
                            None
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if blocks.is_empty() {
                    None
                } else {
                    Some((m.role, blocks))
                }
            }
            MessageContent::Text { content } => {
                let content = content.trim().to_string();
                if content.is_empty() {
                    None
                } else {
                    Some((m.role, content))
                }
            }
        })
        .chunk_by(|m| m.0.clone());
    // merge same role
    let mut msgs = chunks.into_iter().map(|(role, grp)| {
        let txt = grp.into_iter().map(|m| m.1).collect::<Vec<_>>().join("\n");
        (role, txt)
    });
    let first = msgs.next()?;
    // first message does not need prefix
    for (role, text) in msgs {
        let prefix = match role {
            Role::User => "Human: ",
            Role::Assistant => "Assistant: ",
        };
        write!(w, "{}{}{}", line_breaks, prefix, text).unwrap();
    }
    print_out_text(first.1.as_str(), "head.txt");
    print_out_text(w.as_str(), "tail.txt");

    Some(Merged {
        head: first.1,
        tail: w,
        images: imgs,
    })
}
