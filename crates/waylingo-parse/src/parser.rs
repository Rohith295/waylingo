//! The parse state machine: request, validate, repair or retry, finish.

use serde::{Deserialize, Serialize};
use waylingo_http::{HttpRequest, HttpResponse, TransportError};
use waylingo_plan::{Need, Plan, PlanError, SurfaceIntent, TripContext, compile};

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
    /// 429 after all retries.
    LlmRateLimited,
    /// The account's quota or credit is used up (402, or a 429 that says so).
    LlmQuotaExceeded,
    /// Another 4xx: usually a wrong model name, a retired model, or a parameter the
    /// endpoint does not accept. Retrying will not help; check the configuration.
    LlmBadRequest,
    /// The endpoint answered 2xx in a format Waylingo cannot read (for example, the
    /// provider changed its API, or the server is not OpenAI-compatible).
    LlmBadResponse,
    /// 5xx or timeouts after all retries.
    LlmUnavailable,
    /// The endpoint could not be reached (no network, DNS or TLS failure).
    LlmUnreachable,
    /// A bug in Waylingo or in the host driving it (for example, a response for a
    /// request that was never sent).
    Internal,
    /// The model refused the request.
    LlmRefused,
    /// The model reported that the text is not a trip request it can understand.
    /// No repair is attempted: the caller should ask the user to rephrase.
    NotUnderstood,
    /// The request needs the current location, and the host did not pass one.
    NeedsCurrentLocation,
    /// No destination was named and the host passed no active route: ask the user
    /// where to.
    NeedsDestination,
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
    /// The LLM left out the destination and was asked once to add it, if named.
    asked_destination: bool,
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
            asked_destination: false,
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
            return fail(FailureCode::Internal, "no request is pending", vec![]);
        };
        if resp.id != pending.id {
            return fail(
                FailureCode::Internal,
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
            Err(e) => return fail(FailureCode::LlmBadResponse, &e, vec![]),
        };

        let errors = match serde_json::from_str::<SurfaceIntent>(&text) {
            Ok(intent) if intent.error.is_some() => {
                return fail(
                    FailureCode::NotUnderstood,
                    "the request could not be understood as a trip; ask the user to rephrase",
                    vec![],
                );
            }
            Ok(mut intent) => {
                ground_in_query(&mut intent, &self.query);
                // After the destination question, accept only a destination the
                // request names: the LLM must not invent one.
                if self.asked_destination
                    && intent
                        .destination
                        .as_deref()
                        .is_some_and(|d| !named_in(&self.query, d))
                {
                    return fail(
                        FailureCode::NeedsDestination,
                        "no destination was named; ask the user where to",
                        vec![],
                    );
                }
                match compile(&intent, ctx) {
                    Ok(plan) => return ParseStep::Done(Box::new(plan)),
                    Err(errs) => errs,
                }
            }
            Err(e) => vec![PlanError {
                pointer: String::new(),
                reason: format!("not a valid object for the schema: {e}"),
                allowed: vec![],
                needs: None,
            }],
        };

        // Missing context is not repaired: the LLM could only invent it. The one
        // exception: a destination the LLM may have dropped gets one neutral question,
        // and the answer must come from the request text.
        if let Some(need) = errors.iter().find_map(|e| e.needs) {
            if need == Need::Destination
                && !self.asked_destination
                && self.repairs < self.cfg.max_repairs
            {
                self.asked_destination = true;
                self.repairs += 1;
                self.conv.push_reply(text);
                self.conv.push_repair(&[PlanError {
                    pointer: "/destination".into(),
                    reason: DESTINATION_QUESTION.into(),
                    allowed: vec![],
                    needs: None,
                }]);
                return ParseStep::Request(self.next_request());
            }
            let (code, message) = match need {
                Need::Request => (
                    FailureCode::NotUnderstood,
                    "the request could not be understood as a trip; ask the user to rephrase",
                ),
                Need::CurrentLocation => (
                    FailureCode::NeedsCurrentLocation,
                    "the request needs the current location; pass current_location",
                ),
                Need::Destination => (
                    FailureCode::NeedsDestination,
                    "no destination was named; ask the user where to",
                ),
            };
            return fail(code, message, errors);
        }
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
        let quota = resp.status == 402 || (resp.status == 429 && says_quota(resp));
        if !quota && resp.is_retryable() && self.retries < self.cfg.max_retries {
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
        let code = match (resp.transport_error, resp.status) {
            (Some(TransportError::ConnectionFailed), _) => FailureCode::LlmUnreachable,
            (Some(_), _) => FailureCode::LlmUnavailable,
            _ if quota => FailureCode::LlmQuotaExceeded,
            (None, 429) => FailureCode::LlmRateLimited,
            // A redirect means a wrong base URL; other 4xx a wrong model or parameter.
            (None, 300..=499) => FailureCode::LlmBadRequest,
            (None, _) => FailureCode::LlmUnavailable,
        };
        fail(code, &format!("LLM endpoint failed: {what}"), vec![])
    }

    fn next_request(&mut self) -> HttpRequest {
        let req = build_request(&self.conv, &self.cfg, format!("llm-{}", self.calls), 0);
        self.calls += 1;
        self.pending = Some(req.clone());
        req
    }
}

/// Asked once when the LLM's plan has no destination and there is no active route.
const DESTINATION_QUESTION: &str = "missing. If the request names where to go, add it exactly as written there; otherwise leave it out";

/// Whether a destination the LLM gave appears in the request text.
fn named_in(query: &str, destination: &str) -> bool {
    let d = destination.strip_prefix("saved:").unwrap_or(destination);
    let d = d.trim().to_lowercase();
    !d.is_empty() && query.to_lowercase().contains(&d)
}

/// OpenAI reports an exhausted account as a 429 with `insufficient_quota`.
fn says_quota(resp: &HttpResponse) -> bool {
    let body = String::from_utf8_lossy(&resp.body);
    body.contains("insufficient_quota") || body.contains("quota exceeded")
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
        let still_wrong = r#"{"destination":"Utrecht","avoid":["lava"]}"#;
        match p.on_response(&content_reply(&id, still_wrong), &ctx()) {
            ParseStep::Failed(f) => {
                assert_eq!(f.code, FailureCode::InvalidPlan);
                assert!(!f.errors.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 3);
    }

    #[test]
    fn destination_only_is_a_plan() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let reply = content_reply(&r.id, r#"{"destination":"Zeeduinweg 342"}"#);
        match p.on_response(&reply, &ctx()) {
            ParseStep::Done(plan) => assert!(plan.stop_requests.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    fn coffee_stop() -> &'static str {
        r#"{"stops":[{"what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#
    }

    fn step(p: &mut Parser, id: &str, content: &str, ctx: &TripContext) -> ParseStep {
        p.on_response(&content_reply(id, content), ctx)
    }

    #[test]
    fn missing_destination_is_asked_once_then_reported() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let ParseStep::Request(next) = step(&mut p, &r.id, coffee_stop(), &ctx()) else {
            panic!("one neutral question")
        };
        match step(&mut p, &next.id, coffee_stop(), &ctx()) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::NeedsDestination),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 2);
    }

    #[test]
    fn an_invented_destination_is_refused() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let ParseStep::Request(next) = step(&mut p, &r.id, coffee_stop(), &ctx()) else {
            panic!()
        };
        // The request is "Drive home, coffee halfway": it does not name Atlantis.
        let invented = r#"{"destination":"Atlantis","stops":[{"what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#;
        match step(&mut p, &next.id, invented, &ctx()) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::NeedsDestination),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_dropped_destination_named_in_the_request_is_recovered() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let ParseStep::Request(next) = step(&mut p, &r.id, coffee_stop(), &ctx()) else {
            panic!()
        };
        // The request is "Drive home, coffee halfway".
        let named = r#"{"destination":"saved:home","stops":[]}"#;
        match step(&mut p, &next.id, named, &ctx()) {
            ParseStep::Done(plan) => assert!(plan.stop_requests.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_plan_is_not_understood() {
        for reply in ["{}", r#"{"stops":[]}"#] {
            for active in [false, true] {
                let (mut p, r) = start(LlmMode::PromptJson);
                let mut c = ctx();
                if active {
                    c.active_destination = Some(waylingo_plan::Coords {
                        lat: 52.09,
                        lng: 5.12,
                    });
                }
                match step(&mut p, &r.id, reply, &c) {
                    ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::NotUnderstood),
                    other => panic!("{reply} active={active}: {other:?}"),
                }
                assert_eq!(p.calls(), 1);
            }
        }
    }

    #[test]
    fn missing_current_location_fails_without_repair() {
        let (mut p, r) = start(LlmMode::PromptJson);
        let no_location = TripContext {
            current_location: None,
            ..ctx()
        };
        let reply = content_reply(&r.id, r#"{"destination":"Utrecht"}"#);
        match p.on_response(&reply, &no_location) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::NeedsCurrentLocation),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.calls(), 1, "no repair call");
    }

    #[test]
    fn http_failures_have_their_own_codes() {
        let status = |code: u16| {
            let (mut p, r) = start(LlmMode::JsonSchema);
            let resp = HttpResponse {
                id: r.id,
                status: code,
                headers: vec![],
                body: br#"{"error":{"message":"model not found"}}"#.to_vec(),
                transport_error: None,
            };
            match p.on_response(&resp, &ctx()) {
                ParseStep::Failed(f) => f.code,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(status(401), FailureCode::LlmAuth);
        assert_eq!(status(404), FailureCode::LlmBadRequest);
        assert_eq!(status(400), FailureCode::LlmBadRequest);
        assert_eq!(status(301), FailureCode::LlmBadRequest);
        assert_eq!(status(402), FailureCode::LlmQuotaExceeded);

        let (mut p, r) = start(LlmMode::JsonSchema);
        let resp = HttpResponse {
            id: r.id,
            status: 200,
            headers: vec![],
            body: br#"{"output":[{"text":"a new response format"}]}"#.to_vec(),
            transport_error: None,
        };
        match p.on_response(&resp, &ctx()) {
            ParseStep::Failed(f) => assert_eq!(f.code, FailureCode::LlmBadResponse),
            other => panic!("{other:?}"),
        }
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
