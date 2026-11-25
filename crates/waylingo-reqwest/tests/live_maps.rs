//! Live contract tests for the map providers: the whole engine (geocode, route,
//! along-route search, detour ranking, reroute) against the real TomTom and HERE
//! APIs. The LLM step is answered with a fixed plan, so only map keys are needed and
//! a provider-side change (response format, auth, endpoints) is caught on its own.
//! Skipped unless run with `--ignored`; each provider is skipped when its key is not
//! set.
//!
//! ```text
//! TOMTOM_API_KEY=... HERE_API_KEY=... \
//!   cargo test -p waylingo-reqwest --test live_maps -- --ignored --test-threads 1 --nocapture
//! ```
//!
//! `WAYLINGO_LIVE_REQUIRE` (comma-separated: tomtom, here) turns a missing key into a
//! failure, so a scheduled run cannot pass by skipping.
//!
//! Output is statuses, codes and counts only. Place names, addresses and coordinates
//! from the providers are never printed, and keys never are.

use std::time::Duration;

use serde_json::Value;
use waylingo::{
    AuthPlacement, Choices, Coords, CredentialSlot, Engine, EngineConfig, HttpRequest,
    HttpResponse, LlmConfig, LlmMode, ProviderChoice, Session, Step, StopChoice, StopPick,
    TransportError, TripContext, TripRequest,
};

/// "Drive to Utrecht, coffee halfway", as the LLM would parse it.
const COFFEE_HALFWAY: &str = r#"{"destination":"Utrecht","stops":[{"id":"coffee","what":{"category":"coffee"},"anchor":{"kind":"fraction","value":0.5}}]}"#;
/// "Drive to Utrecht".
const DESTINATION_ONLY: &str = r#"{"destination":"Utrecht","stops":[]}"#;

struct Live {
    client: reqwest::blocking::Client,
    key: String,
    /// The fixed plan returned for every LLM request.
    plan: &'static str,
}

impl Live {
    fn new(key: &str, plan: &'static str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        Self {
            client,
            key: key.to_string(),
            plan,
        }
    }

    fn answer(&self, req: &HttpRequest) -> HttpResponse {
        if req.credential == CredentialSlot::Llm {
            let body = serde_json::json!({"choices": [{"message": {"content": self.plan}}]});
            return HttpResponse {
                id: req.id.clone(),
                status: 200,
                headers: vec![],
                body: serde_json::to_vec(&body).expect("json"),
                transport_error: None,
            };
        }
        // Stay under TomTom's default 5 requests per second.
        std::thread::sleep(Duration::from_millis(250 + u64::from(req.not_before_ms)));
        let mut url = reqwest::Url::parse(&req.url).expect("valid url");
        let mut b = match &req.auth {
            AuthPlacement::QueryParam { name } => {
                url.query_pairs_mut().append_pair(name, &self.key);
                self.build(req, url)
            }
            AuthPlacement::Header { name, prefix } => self
                .build(req, url)
                .header(name, format!("{prefix}{}", self.key)),
        };
        for h in &req.headers {
            b = b.header(&h.name, &h.value);
        }
        match b.send() {
            Ok(r) => HttpResponse {
                id: req.id.clone(),
                status: r.status().as_u16(),
                headers: vec![],
                body: r.bytes().map(|b| b.to_vec()).unwrap_or_default(),
                transport_error: None,
            },
            Err(e) => HttpResponse {
                id: req.id.clone(),
                status: 0,
                headers: vec![],
                body: vec![],
                transport_error: Some(if e.is_timeout() {
                    TransportError::Timeout
                } else {
                    TransportError::ConnectionFailed
                }),
            },
        }
    }

    fn build(&self, req: &HttpRequest, url: reqwest::Url) -> reqwest::blocking::RequestBuilder {
        if req.method == "POST" {
            self.client
                .post(url)
                .body(req.body.clone().unwrap_or_default())
        } else {
            self.client.get(url)
        }
    }

    fn drive(&self, mut session: Session, mut step: Step) -> Value {
        loop {
            match step {
                Step::Done { geojson } => return serde_json::from_str(&geojson).expect("json"),
                Step::NeedRequests { requests } => {
                    let resps = requests.iter().map(|r| self.answer(r)).collect();
                    step = session.step(resps).expect("step");
                }
            }
        }
    }
}

/// The key for a provider, or `None` (skip) unless it is required.
fn key(name: &str) -> Option<String> {
    let var = format!("{}_API_KEY", name.to_uppercase());
    let v = std::env::var(&var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if v.is_none() {
        let required = std::env::var("WAYLINGO_LIVE_REQUIRE").unwrap_or_default();
        assert!(
            !required.split(',').any(|s| s.trim() == name),
            "{var} is not set but {name} is required"
        );
        println!("SKIP {name}: {var} is not set");
    }
    v
}

fn engine(provider: &str) -> Engine {
    let choice = match provider {
        "tomtom" => ProviderChoice::TomTom { base_url: None },
        "here" => ProviderChoice::Here { base_url: None },
        _ => unreachable!("unknown provider {provider}"),
    };
    // The LLM endpoint is never called: `Live` answers LLM requests itself.
    let llm = LlmConfig::new("https://llm.invalid/v1", "fixed-plan", LlmMode::JsonSchema);
    Engine::new(EngineConfig::new(llm, choice))
}

fn trip() -> TripRequest {
    let mut context = TripContext {
        current_location: Some(Coords {
            lat: 52.3702,
            lng: 4.8952,
        }),
        locale: Some("en-GB".into()),
        ..TripContext::default()
    };
    context.defaults.top_n = 3;
    TripRequest {
        query: "Drive to Utrecht, coffee halfway".into(),
        context,
        request_id: None,
    }
}

fn kinds<'a>(v: &'a Value, kind: &str) -> Vec<&'a Value> {
    v["features"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter(|f| f["properties"]["kind"] == kind)
                .collect()
        })
        .unwrap_or_default()
}

fn summary(label: &str, v: &Value) {
    println!(
        "{label}: status {}, error {}, stops {}, unresolved {}, map calls {}, warnings {}",
        v["status"],
        v["error"]["code"],
        kinds(v, "stop").len(),
        kinds(v, "unresolved_stop").len(),
        v["usage"]["map_calls"],
        v["warnings"].as_array().map_or(0, Vec::len),
    );
}

fn check_route(v: &Value) {
    let route = kinds(v, "route");
    assert_eq!(route.len(), 1, "one route feature");
    assert_eq!(route[0]["geometry"]["type"], "LineString");
    let km = route[0]["properties"]["total_m"].as_f64().unwrap_or(0.0) / 1000.0;
    assert!(
        (25.0..=90.0).contains(&km),
        "route length plausible ({km:.0} km)"
    );
    assert!(
        v["attribution"].as_str().is_some_and(|a| !a.is_empty()),
        "attribution is present"
    );
}

fn plan_and_reroute(provider: &str) {
    let Some(key) = key(provider) else { return };
    let live = Live::new(&key, COFFEE_HALFWAY);
    let engine = engine(provider);
    let (session, step) = engine.start(trip()).expect("start");
    let v = live.drive(session, step);
    summary(&format!("{provider} plan"), &v);
    assert_ne!(v["status"], "failed", "{}", v["error"]);
    assert_eq!(
        kinds(&v, "stop").len() + kinds(&v, "unresolved_stop").len(),
        1
    );
    check_route(&v);

    let Some(stop) = kinds(&v, "stop").into_iter().find(|s| {
        s["properties"]["alternatives"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    }) else {
        println!("{provider}: no alternatives returned; reroute not exercised");
        return;
    };
    let alt = stop["properties"]["alternatives"][0]["provider_place_id"]
        .as_str()
        .expect("alternative id")
        .to_string();
    let choices = Choices {
        locations: vec![],
        stops: vec![StopChoice {
            stop_request_id: "coffee".into(),
            occurrence: 0,
            pick: StopPick::Place {
                provider_place_id: alt.clone(),
            },
        }],
    };
    let (session, step) = engine
        .resume(&v.to_string(), choices, None)
        .expect("resume");
    let again = live.drive(session, step);
    summary(&format!("{provider} reroute"), &again);
    assert_ne!(again["status"], "failed", "{}", again["error"]);
    assert_eq!(again["usage"]["llm_calls"], 0);
    assert_eq!(again["usage"]["map_calls"], 1);
    let picked = &kinds(&again, "stop")[0]["properties"]["pick"]["provider_place_id"];
    assert_eq!(picked, alt.as_str(), "the chosen place is the pick");
    check_route(&again);
}

fn destination_only(provider: &str) {
    let Some(key) = key(provider) else { return };
    let live = Live::new(&key, DESTINATION_ONLY);
    let (session, step) = engine(provider).start(trip()).expect("start");
    let v = live.drive(session, step);
    summary(&format!("{provider} destination only"), &v);
    assert_eq!(v["status"], "complete", "{}", v["error"]);
    assert_eq!(v["usage"]["map_calls"], 2, "geocode and route only");
    check_route(&v);
}

fn wrong_key(provider: &str) {
    if key(provider).is_none() {
        return;
    }
    let live = Live::new("not-a-real-key", COFFEE_HALFWAY);
    let (session, step) = engine(provider).start(trip()).expect("start");
    let v = live.drive(session, step);
    summary(&format!("{provider} wrong key"), &v);
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["code"], "PROVIDER_AUTH");
}

#[test]
#[ignore = "live: TomTom"]
fn tomtom_contract() {
    plan_and_reroute("tomtom");
    destination_only("tomtom");
    wrong_key("tomtom");
}

#[test]
#[ignore = "live: HERE"]
fn here_contract() {
    plan_and_reroute("here");
    destination_only("here");
    wrong_key("here");
}
