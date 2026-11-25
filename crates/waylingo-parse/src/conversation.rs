//! The message history sent to the LLM. Its constructor and methods are the only
//! way to add messages, and each accepts only an allowed source type.

use serde::Serialize;
use waylingo_plan::PlanError;

use crate::ParseInput;
use crate::prompt::{SYSTEM_PROMPT, repair_message};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Message {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Conversation {
    messages: Vec<Message>,
}

impl Conversation {
    pub(crate) fn new(input: &ParseInput, system_suffix: Option<&str>) -> Self {
        Self::with_system(input, SYSTEM_PROMPT, system_suffix)
    }

    pub(crate) fn with_system(input: &ParseInput, base: &str, system_suffix: Option<&str>) -> Self {
        let mut system = base.to_string();
        if let Some(s) = system_suffix {
            system.push_str(s);
        }
        Self {
            messages: vec![
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: input.user_message(),
                },
            ],
        }
    }

    /// The LLM's own reply, echoed back so the repair has context.
    pub(crate) fn push_reply(&mut self, raw_reply: String) {
        self.messages.push(Message {
            role: "assistant",
            content: raw_reply,
        });
    }

    pub(crate) fn push_repair(&mut self, errors: &[PlanError]) {
        self.messages.push(Message {
            role: "user",
            content: repair_message(errors),
        });
    }

    pub(crate) fn messages(&self) -> &[Message] {
        &self.messages
    }
}
