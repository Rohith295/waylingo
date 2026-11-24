use serde::{Deserialize, Serialize};

/// How the LLM is asked for structured output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmMode {
    /// `response_format: {type: "json_schema"}`. OpenAI and most hosted endpoints.
    JsonSchema,
    /// A single forced function call whose parameters are the schema.
    ToolCall,
    /// Schema in the prompt; JSON is extracted from the reply text. Last resort for
    /// local servers without structured output.
    PromptJson,
}

/// LLM endpoint settings. Holds no key: the host adds it to each request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// OpenAI-compatible base URL, e.g. `https://api.openai.com/v1` or
    /// `http://localhost:11434/v1`. `/chat/completions` is appended.
    pub base_url: String,
    pub model: String,
    pub mode: LlmMode,
    /// How many times invalid output is sent back for correction. Default 2.
    pub max_repairs: u8,
    /// How many times a 429, 5xx or transport failure is retried. Default 2.
    pub max_retries: u8,
    /// Per-request timeout hint for the host. Default 20 000 ms.
    pub timeout_ms: u32,
}

impl LlmConfig {
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, mode: LlmMode) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            mode,
            max_repairs: 2,
            max_retries: 2,
            timeout_ms: 20_000,
        }
    }

    pub(crate) fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}
