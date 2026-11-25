//! Settings shared by the examples: `.env` loading and LLM configuration.
//! Keys are read here and never printed.

use std::collections::HashMap;

use waylingo::{LlmConfig, LlmMode, PromptStyle, ProviderChoice};

pub fn load_dotenv() -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(".env") {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                m.insert(k.trim().to_string(), v.to_string());
            }
        }
    }
    m
}

pub fn setting(dotenv: &HashMap<String, String>, name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| dotenv.get(name).cloned())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn llm_config(env: &HashMap<String, String>) -> Result<LlmConfig, String> {
    let provider = setting(env, "LLM_PROVIDER")
        .unwrap_or_else(|| "anthropic".into())
        .to_lowercase();
    let (base, model, mode) = match provider.as_str() {
        "anthropic" | "claude" => (
            "https://api.anthropic.com/v1",
            Some("claude-sonnet-5-5"),
            LlmMode::ToolCall,
        ),
        "openai" => ("https://api.openai.com/v1", None, LlmMode::JsonSchema),
        "ollama" | "local" => ("http://localhost:11434/v1", None, LlmMode::PromptJson),
        other => {
            return Err(format!(
                "unknown LLM_PROVIDER {other:?}; use anthropic, openai or ollama"
            ));
        }
    };
    let base = setting(env, "LLM_BASE_URL").unwrap_or_else(|| base.to_string());
    let model = setting(env, "LLM_MODEL")
        .or_else(|| model.map(String::from))
        .ok_or_else(|| format!("set LLM_MODEL for LLM_PROVIDER={provider}"))?;
    let mode = match setting(env, "LLM_MODE").as_deref() {
        None => mode,
        Some("json_schema") => LlmMode::JsonSchema,
        Some("tool_call") => LlmMode::ToolCall,
        Some("prompt_json") => LlmMode::PromptJson,
        Some(other) => return Err(format!("unknown LLM_MODE {other:?}")),
    };
    let prompt = match setting(env, "LLM_PROMPT").as_deref() {
        None | Some("full") => PromptStyle::Full,
        Some("compact") => PromptStyle::Compact,
        Some(other) => return Err(format!("unknown LLM_PROMPT {other:?}; use full or compact")),
    };
    println!("LLM: {provider}, model {model}, mode {mode:?}, prompt {prompt:?}");
    let mut cfg = LlmConfig::new(base, model, mode);
    cfg.prompt = prompt;
    Ok(cfg)
}

/// The map provider from `MAP_PROVIDER` (`tomtom`, the default, or `here`) and its key
/// (`TOMTOM_API_KEY` or `HERE_API_KEY`).
pub fn map_provider(env: &HashMap<String, String>) -> Result<(ProviderChoice, String), String> {
    let name = setting(env, "MAP_PROVIDER")
        .unwrap_or_else(|| "tomtom".into())
        .to_lowercase();
    let (choice, key_name) = match name.as_str() {
        "tomtom" => (ProviderChoice::TomTom { base_url: None }, "TOMTOM_API_KEY"),
        "here" => (ProviderChoice::Here { base_url: None }, "HERE_API_KEY"),
        other => {
            return Err(format!(
                "unknown MAP_PROVIDER {other:?}; use tomtom or here"
            ));
        }
    };
    let key = setting(env, key_name)
        .ok_or_else(|| format!("{key_name} must be set for MAP_PROVIDER={name}"))?;
    println!("Map provider: {name}");
    Ok((choice, key))
}
