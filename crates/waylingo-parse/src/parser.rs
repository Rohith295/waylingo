//! The parse state machine: request, validate, repair or retry, finish.

use serde::{Deserialize, Serialize};
use waylingo_http::{HttpRequest, HttpResponse};
use waylingo_plan::{Plan, PlanError, SurfaceIntent, TripContext, compile};

use crate::conversation::Conversation;
use crate::ground::ground_in_query;
use crate::openai::{Reply, build_request, extract, llm_schema};
use crate::prompt::{COMPACT_SYSTEM_PROMPT, SCHEMA_PREAMBLE};
use crate::{LlmConfig, LlmMode, ParseInput, PromptStyle};

/// Longest backoff the parser asks for, in milliseconds.
const MAX_BACKOFF_MS: u32 = 30_000;

/// Why parsing ended without a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FailureCode {
    /// The LLM's output was still invalid after all repairs.
    InvalidPlan,
    /// 401 or 403 from the LLM endpoint.
    LlmAuth,
    /// 429, 5xx or transport failures after all retries, or an unusable response.
    LlmUnavailable,
    /// The model refused the request.
    LlmRefused,
    /// The model reported that the text is not a trip request it can understand.
    /// No repair is attempted: the caller should ask the user to rephrase.
    NotUnderstood,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseFailure {
    pub code: FailureCode,
    pub message: String,
    /// The last validation errors, for `InvalidPlan`.
    pub errors: Vec<PlanError>,
}

/// What the host must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseStep {
    /// Send this request and pass the response to [`Parser::on_response`].
    Request(HttpRequest),
    Done(Box<Plan>),
    Failed(ParseFailure),
}

/// Drives one parse. Single use.
#[derive(Debug)]
pub struct Parser {
    cfg: LlmConfig,
    conv: Conversation,
    /// The user's request, used to snap garbled place names back to their words.
    query: String,
    repairs: u8,
    retries: u8,
    calls: u32,
    pending: Option<HttpRequest>,
}

impl Parser {
    /// Start parsing. Returns the first LLM request.
    #[must_use]
    pub fn start(input: &ParseInput, cfg: LlmConfig) -> (Self, HttpRequest) {
        let conv = match cfg.prompt {
            PromptStyle::Compact => Conversation::with_system(input, COMPACT_SYSTEM_PROMPT, None),
            PromptStyle::Full => {
                let suffix = (cfg.mode == LlmMode::PromptJson)
                    .then(|| format!("{SCHEMA_PREAMBLE}{}", llm_schema()));
                Conversation::new(input, suffix.as_deref())
            }
        };
        let mut p = Self {
            cfg,
            conv,
            query: input.query().to_string(),
            repairs: 0,
            retries: 0,
            calls: 0,
            pending: None,
        };
        let req = p.next_request();
        (p, req)
    }

    /// Number of LLM requests issued so far, retries included.
    #[must_use]
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// Handle the host's response to the last request.
    ///
    /// `ctx` is used only to compile the plan locally. Nothing from it is sent to the
    /// LLM: compile errors refer to fields of the LLM's own JSON.
    pub fn on_response(&mut self, resp: &HttpResponse, ctx: &TripContext) -> ParseStep {
        let Some(pending) = self.pending.take() else {
            return fail(FailureCode::LlmUnavailable, "no request is pending", vec![]);
        };
        if resp.id != pending.id {
            return fail(
                FailureCode::LlmUnavailable,
                "response id does not match the pending request",
                vec![],
            );
        }
        if resp.transport_error.is_some() || !(200..300).contains(&resp.status) {
            return self.on_http_failure(resp, pending);
        }

        let text = match extract(resp, self.cfg.mode) {
            Ok(Reply::Json(t)) => t,
            Ok(Reply::Refused(r)) => return fail(FailureCode::LlmRefused, &r, vec![]),
            Err(e) => return fail(FailureCode::LlmUnavailable, &e, vec![]),
        };

        let errors = match serde_json::from_str::<SurfaceIntent>(&text) {
            Ok(intent) if intent.error.is_some() => {
                return fail(
                    FailureCode::NotUnderstood,
                    "the request could not be understood as a trip with stops; ask the user to rephrase",
                    vec![],
                );
            }
            Ok(mut intent) => {
                ground_in_query(&mut intent, &self.query);
                match compile(&intent, ctx) {
                    Ok(plan) => return ParseStep::Done(Box::new(plan)),
                    Err(errs) => errs,
                }
            }
            Err(e) => vec![PlanError {
                pointer: String::new(),
                reason: format!("not a valid object for the schema: {e}"),
                allowed: vec![],
            }],
        };

        if self.repairs >= self.cfg.max_repairs {
            return fail(
                FailureCode::InvalidPlan,
                "the plan was still invalid after all repairs",
                errors,
            );
        }
        self.repairs += 1;
        self.conv.push_reply(text);
        self.conv.push_repair(&errors);
        ParseStep::Request(self.next_request())
    }

    fn on_http_failure(&mut self, resp: &HttpResponse, pending: HttpRequest) -> ParseStep {
        if matches!(resp.status, 401 | 403) {
            return fail(
                FailureCode::LlmAuth,
                &format!(
                    "LLM endpoint returned {}{}",
                    resp.status,
                    error_detail(resp)
                ),
                vec![],
            );
        }
        if resp.is_retryable() && self.retries < self.cfg.max_retries {
            self.retries += 1;
            let backoff = 1_000_u32 << (self.retries - 1);
            let mut again = pending;
            again.id = format!("llm-{}", self.calls);
            again.not_before_ms = retry_after_ms(resp).unwrap_or(backoff).min(MAX_BACKOFF_MS);
            self.calls += 1;
            self.pending = Some(again.clone());
            return ParseStep::Request(again);
        }
        let what = resp.transport_error.map_or_else(
            || format!("HTTP {}{}", resp.status, error_detail(resp)),
            |e| format!("{e:?}"),
        );
        fail(
            FailureCode::LlmUnavailable,
            &format!("LLM endpoint failed: {what}"),
            vec![],
        )
    }

    fn next_request(&mut self) -> HttpRequest {
        let req = build_request(&self.conv, &self.cfg, format!("llm-{}", self.calls), 0);
        self.calls += 1;
        self.pending = Some(req.clone());
        req
    }
}

fn fail(code: FailureCode, message: &str, errors: Vec<PlanError>) -> ParseStep {
    ParseStep::Failed(ParseFailure {
        code,
        message: message.to_string(),
        errors,
    })
}

fn retry_after_ms(resp: &HttpResponse) -> Option<u32> {
    let secs: f64 = resp.header("Retry-After")?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| (secs * 1000.0).min(f64::from(MAX_BACKOFF_MS)) as u32)
}

/// The endpoint's own error text (OpenAI-style `{"error":{"message":...}}`), cut to
/// 300 characters. Error bodies describe the request, not secrets, but anything that
/// looks like a key is masked anyway.
fn error_detail(resp: &HttpResponse) -> String {
    let v: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    let msg = v["error"]["message"]
        .as_str()
        .or_else(|| v["message"].as_str())
        .or_else(|| v["error"].as_str());
    let Some(msg) = msg else { return String::new() };
    let masked: Vec<String> = msg
        .split_whitespace()
        .map(|w| {
            if w.starts_with("sk-") || w.len() > 40 {
                "<redacted>".into()
            } else {
                w.to_string()
            }
        })
        .collect();
    let mut s = masked.join(" ");
    if s.chars().count() > 300 {
        s = s.chars().take(300).collect::<String>() + "...";
    }
    format!(" ({s})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use waylingo_http::{CredentialSlot, Header, TransportError};
    use waylingo_plan::{Coords, Location, SavedPlace};

    const CANARY_COORDS: [&str; 4] = ["52.3702", "4.8952", "52.0907", "5.1214"];

    fn ctx() -> TripContext {
        TripContext {
            current_location: Some(Coords {
                lat: 52.3702,
                lng: 4.8952,
            }),
            saved_places: vec![SavedPlace {
                name: "home".into(),
                location: Coords {
                    lat: 52.0907,
                    lng: 5.1214,
                },
            }],
            ..TripContext::default()
        }
    }

    fn start(mode: LlmMode) -> (Parser, HttpRequest) {
        let input = ParseInput::new("Drive home, coffee halfway", &ctx()).unwrap();
        Parser::start(&input, LlmConfig::new("https://llm.example/v1", "m", mode))
    }

    fn content_reply(id: &str, content: &str) -> HttpResponse {
        let body = json!({"choices":[{"message":{"content": content}}]});
        HttpResponse {
            id: id.into(),
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&body).unwrap(),
            transport_error: None,
        }
    }

    fn status(id: &str, s: u16, headers: Vec<Header>) -> HttpResponse {
        HttpResponse {
            id: id.into(),
            status: s,
            headers,
            body: vec![],
            transport_error: None,
        }
    }

    fn assert_no_coords(r: &HttpRequest) {
        let b = String::from_utf8(r.body.clone().unwrap()).unwrap();
        for c in CANARY_COORDS {
            assert!(!b.contains(c), "coordinate {c} reached the LLM request");
        }
    }

    const GOOD: &str = r#"{"destination":"saved:home","stops":[{"what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#;

    #[test]
    fn happy_path_resolves_saved_place_locally() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        assert_eq!(r.credential, CredentialSlot::Llm);
        assert_no_coords(&r);
        match p.on_response(&content_reply(&r.id, GOOD), &ctx()) {
            ParseStep::Done(plan) => {
                assert!(
                    matches!(plan.destination, Location::Coords { lat, .. } if (lat - 52.0907).abs() < 1e-9)
                );
                assert!(matches!(plan.origin, Location::Coords { .. }));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 1);
    }

    #[test]
    fn repair_sends_errors_and_own_reply_only() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        let bad = r#"{"destination":"saved:home","stops":[{"what":{"category":"cafe"},"anchor":{"kind":"fraction","value":0.5}}]}"#;
        let ParseStep::Request(r2) = p.on_response(&content_reply(&r.id, bad), &ctx()) else {
            panic!()
        };
        assert_ne!(r2.id, r.id);
        assert_no_coords(&r2);
        let b: Value = serde_json::from_slice(r2.body.as_ref().unwrap()).unwrap();
        let msgs = b["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2]["role"], "assistant");
        let repair = msgs[3]["content"].as_str().unwrap();
        assert!(repair.contains("/stops/0/what/category"));
        assert!(repair.contains("coffee"), "allowed values listed");
        assert!(matches!(
            p.on_response(&content_reply(&r2.id, GOOD), &ctx()),
            ParseStep::Done(_)
        ));
        assert_eq!(p.calls(), 2);
    }

    #[test]
    fn gives_up_after_max_repairs() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let mut id = r.id;
        for _ in 0..2 {
            let ParseStep::Request(next) =
                p.on_response(&content_reply(&id, "not json at all"), &ctx())
            else {
                panic!()
            };
            id = next.id;
        }
        match p.on_response(&content_reply(&id, "{}"), &ctx()) {
            ParseStep::Failed(f) => {
                assert_eq!(f.code, FailureCode::InvalidPlan);
                assert!(!f.errors.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 3);
    }

    #[test]
    fn retries_429_with_retry_after_then_fails() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        let ra = vec![Header {
            name: "retry-after".into(),
            value: "3".into(),
        }];
        let ParseStep::Request(r2) = p.on_response(&status(&r.id, 429, ra), &ctx()) else {
            panic!()
        };
        assert_eq!(r2.not_before_ms, 3_000);
        assert_eq!(r2.body, r.body, "same request body on retry");
        let ParseStep::Request(r3) = p.on_response(&status(&r2.id, 503, vec![]), &ctx()) else {
            panic!()
        };
        assert_eq!(
            r3.not_before_ms, 2_000,
            "exponential backoff without Retry-After"
        );
        match p.on_response(&status(&r3.id, 503, vec![]), &ctx()) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::LlmUnavailable),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn transport_errors_retry() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        let resp = HttpResponse {
            transport_error: Some(TransportError::Timeout),
            ..status(&r.id, 0, vec![])
        };
        assert!(matches!(
            p.on_response(&resp, &ctx()),
            ParseStep::Request(_)
        ));
    }

    #[test]
    fn auth_and_refusal_fail_fast() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        assert!(matches!(
            p.on_response(&status(&r.id, 401, vec![]), &ctx()),
            ParseStep::Failed(ParseFailure {
                code: FailureCode::LlmAuth,
                ..
            })
        ));
        let (mut p, r) = start(LlmMode::JsonSchema);
        let body = json!({"choices":[{"message":{"content":null,"refusal":"cannot help"}}]});
        let resp = HttpResponse {
            body: serde_json::to_vec(&body).unwrap(),
            ..status(&r.id, 200, vec![])
        };
        assert!(matches!(
            p.on_response(&resp, &ctx()),
            ParseStep::Failed(ParseFailure {
                code: FailureCode::LlmRefused,
                ..
            })
        ));
    }

    #[test]
    fn wrong_id_and_use_after_finish_are_rejected() {
        let (mut p, _) = start(LlmMode::JsonSchema);
        assert!(matches!(
            p.on_response(&content_reply("other", GOOD), &ctx()),
            ParseStep::Failed(_)
        ));
        let (mut p, r) = start(LlmMode::JsonSchema);
        assert!(matches!(
            p.on_response(&content_reply(&r.id, GOOD), &ctx()),
            ParseStep::Done(_)
        ));
        assert!(matches!(
            p.on_response(&content_reply(&r.id, GOOD), &ctx()),
            ParseStep::Failed(_)
        ));
    }

    #[test]
    fn not_understood_fails_fast_without_repair() {
        let (mut p, r) = start(LlmMode::JsonSchema);
        let reply = content_reply(&r.id, r#"{"error":"not_understood","stops":[]}"#);
        match p.on_response(&reply, &ctx()) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::NotUnderstood),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 1, "no repair round");
    }

    #[test]
    fn prompt_json_mode_embeds_schema_in_system_prompt() {
        let (_, r) = start(LlmMode::PromptJson);
        let b: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
        let sys = b["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("JSON Schema"));
        assert!(sys.contains("\"stops\""));
        assert!(b.get("response_format").is_none());
    }
}
