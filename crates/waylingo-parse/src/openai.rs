//! OpenAI chat-completions request building and response parsing.

use serde_json::{Value, json};
use waylingo_http::{AuthPlacement, CredentialSlot, Header, HttpRequest, HttpResponse};

use crate::conversation::Conversation;
use crate::{LlmConfig, LlmMode};

pub(crate) const TOOL_NAME: &str = "submit_trip_intent";

/// The schema as sent to the LLM: the `$schema` meta key removed, everything else kept.
pub(crate) fn llm_schema() -> Value {
    let mut s = waylingo_plan::surface_intent_schema();
    if let Some(o) = s.as_object_mut() {
        o.remove("$schema");
    }
    s
}

pub(crate) fn build_request(
    conv: &Conversation,
    cfg: &LlmConfig,
    id: String,
    not_before_ms: u32,
) -> HttpRequest {
    let mut body = json!({
        "model": cfg.model,
        "messages": conv.messages(),
    });
    if let Some(t) = cfg.temperature {
        body["temperature"] = json!(t);
    }
    match cfg.mode {
        LlmMode::JsonSchema => {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": { "name": "surface_intent", "strict": false, "schema": llm_schema() }
            });
        }
        LlmMode::ToolCall => {
            body["tools"] = json!([{
                "type": "function",
                "function": {
                    "name": TOOL_NAME,
                    "description": "Submit the structured trip request. Always call this tool.",
                    "parameters": llm_schema()
                }
            }]);
            // "auto", not a forced function: some endpoints (Anthropic's compatibility
            // layer with some models) reject forced tool choice. If the model answers
            // in text instead, `extract` reads the JSON from the content.
            body["tool_choice"] = json!("auto");
        }
        LlmMode::PromptJson => {}
    }
    HttpRequest {
        id,
        method: "POST".into(),
        url: cfg.chat_url(),
        headers: vec![Header {
            name: "Content-Type".into(),
            value: "application/json".into(),
        }],
        body: Some(serde_json::to_vec(&body).unwrap_or_default()),
        credential: CredentialSlot::Llm,
        auth: AuthPlacement::Header {
            name: "Authorization".into(),
            prefix: "Bearer ".into(),
        },
        timeout_ms: cfg.timeout_ms,
        not_before_ms,
    }
}

/// What came back from a successful (2xx) call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// The text that should hold the JSON object.
    Json(String),
    /// The model declined (OpenAI `refusal` field).
    Refused(String),
}

/// Extract the JSON text from a 2xx chat-completions body.
pub(crate) fn extract(resp: &HttpResponse, mode: LlmMode) -> Result<Reply, String> {
    let v: Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("response is not JSON: {e}"))?;
    let msg = &v["choices"][0]["message"];
    if msg.is_null() {
        return Err("response has no choices[0].message".into());
    }
    if let Some(r) = msg["refusal"].as_str().filter(|r| !r.is_empty()) {
        return Ok(Reply::Refused(r.to_string()));
    }
    let text = match mode {
        LlmMode::ToolCall => msg["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .map(String::from)
            // Some servers answer in content even when a tool is forced.
            .or_else(|| msg["content"].as_str().map(String::from)),
        LlmMode::JsonSchema | LlmMode::PromptJson => msg["content"].as_str().map(String::from),
    }
    .ok_or("response has no content or tool call arguments")?;
    Ok(Reply::Json(json_object_text(&text)))
}

/// Strip code fences and surrounding prose: keep the outermost `{ ... }`.
pub(crate) fn json_object_text(text: &str) -> String {
    let t = text.trim();
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if b > a => t[a..=b].to_string(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParseInput;
    use waylingo_plan::TripContext;

    fn conv() -> Conversation {
        Conversation::new(
            &ParseInput::new("coffee in 20 min", &TripContext::default()).unwrap(),
            None,
        )
    }

    fn body(r: &HttpRequest) -> Value {
        serde_json::from_slice(r.body.as_ref().unwrap()).unwrap()
    }

    fn ok(v: Value) -> HttpResponse {
        HttpResponse {
            id: "x".into(),
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&v).unwrap(),
            transport_error: None,
        }
    }

    #[test]
    fn json_schema_request_shape() {
        let cfg = LlmConfig::new("https://api.example.com/v1/", "m1", LlmMode::JsonSchema);
        let r = build_request(&conv(), &cfg, "llm-0".into(), 0);
        assert_eq!(r.url, "https://api.example.com/v1/chat/completions");
        assert_eq!(r.credential, CredentialSlot::Llm);
        assert!(!r.url.contains("key"));
        let b = body(&r);
        assert_eq!(b["model"], "m1");
        assert!(b.get("temperature").is_none(), "left out unless configured");
        let mut cfg0 = cfg.clone();
        cfg0.temperature = Some(0.0);
        let b0 = body(&build_request(&conv(), &cfg0, "llm-1".into(), 0));
        assert_eq!(b0["temperature"], 0.0);
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert!(b["response_format"]["json_schema"]["schema"]["properties"]["stops"].is_object());
        assert!(
            b["response_format"]["json_schema"]["schema"]
                .get("$schema")
                .is_none()
        );
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["role"], "user");
    }

    #[test]
    fn tool_call_request_shape() {
        let cfg = LlmConfig::new("http://localhost:11434/v1", "llama", LlmMode::ToolCall);
        let b = body(&build_request(&conv(), &cfg, "llm-0".into(), 0));
        assert_eq!(b["tools"][0]["function"]["name"], TOOL_NAME);
        assert_eq!(b["tool_choice"], "auto");
        assert!(b.get("response_format").is_none());
    }

    #[test]
    fn extracts_each_mode() {
        let content =
            ok(json!({"choices":[{"message":{"content":"```json\n{\"stops\":[]}\n```"}}]}));
        assert_eq!(
            extract(&content, LlmMode::PromptJson).unwrap(),
            Reply::Json("{\"stops\":[]}".into())
        );
        assert_eq!(
            extract(&content, LlmMode::JsonSchema).unwrap(),
            Reply::Json("{\"stops\":[]}".into())
        );
        let tool = ok(
            json!({"choices":[{"message":{"content":null,"tool_calls":[{"function":{"name":TOOL_NAME,"arguments":"{\"a\":1}"}}]}}]}),
        );
        assert_eq!(
            extract(&tool, LlmMode::ToolCall).unwrap(),
            Reply::Json("{\"a\":1}".into())
        );
        let refusal = ok(json!({"choices":[{"message":{"content":null,"refusal":"no"}}]}));
        assert_eq!(
            extract(&refusal, LlmMode::JsonSchema).unwrap(),
            Reply::Refused("no".into())
        );
        assert!(extract(&ok(json!({"choices":[]})), LlmMode::JsonSchema).is_err());
        let not_json = HttpResponse {
            body: b"<html>".to_vec(),
            ..ok(json!({}))
        };
        assert!(extract(&not_json, LlmMode::JsonSchema).is_err());
    }
}
