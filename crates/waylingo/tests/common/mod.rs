//! Shared test harness: a fake LLM and TomTom fixtures, no network.
#![allow(dead_code)]

use serde_json::{Value, json};
use waylingo::{
    Coords, CredentialSlot, Engine, EngineConfig, HttpRequest, HttpResponse, LlmConfig, LlmMode,
    ProviderChoice, Session, Step, TripContext, TripRequest,
};

pub const LLM_PLAN: &str = r#"{"destination":"Berlin","stops":[
  {"id":"charger","what":{"category":"ev_charging","min_power_kw":150},"anchor":{"kind":"every","amount":{"value":200,"unit":"km"}}},
  {"id":"food","what":{"category":"fast_food","brand":"McDonald's"},"anchor":{"kind":"offset","from":"destination","direction":"before","amount":{"value":15,"unit":"min"}}}]}"#;

/// Three McDonald's near the "15 min before arrival" anchor, best detour first.
pub const MCD: &str = r#"{"results":[
  {"id":"synthetic-mcd-1","poi":{"name":"WAYLINGO_CANARY_9 Burgers","brands":[{"name":"McDonald's"}]},"position":{"lat":52.49,"lon":13.33},"detourTime":180,"detourDistance":900},
  {"id":"synthetic-mcd-2","poi":{"name":"WAYLINGO_CANARY_10 Burgers","brands":[{"name":"McDonald's"}]},"position":{"lat":52.50,"lon":13.35},"detourTime":240,"detourDistance":1500},
  {"id":"synthetic-mcd-3","poi":{"name":"WAYLINGO_CANARY_11 Burgers","brands":[{"name":"McDonald's"}]},"position":{"lat":52.48,"lon":13.30},"detourTime":300,"detourDistance":2100}]}"#;

pub fn fixture(name: &str) -> Vec<u8> {
    let p = format!(
        "{}/../waylingo-providers/tests/fixtures/tomtom/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

pub fn engine() -> Engine {
    Engine::new(EngineConfig::new(
        LlmConfig::new("https://llm.example/v1", "test-model", LlmMode::JsonSchema),
        ProviderChoice::TomTom { base_url: None },
    ))
}

pub fn request() -> TripRequest {
    TripRequest {
        query: "Drive to Berlin, a fast charger every 200 km and a McDonald's 15 minutes before we arrive".into(),
        context: TripContext {
            current_location: Some(Coords { lat: 52.3702, lng: 4.8952 }),
            ..TripContext::default()
        },
        request_id: Some("req-1".into()),
    }
}

pub fn reply(r: &HttpRequest, status: u16, body: Vec<u8>) -> HttpResponse {
    HttpResponse {
        id: r.id.clone(),
        status,
        headers: vec![],
        body,
        transport_error: None,
    }
}

/// A fake host. `llm` answers LLM calls; map calls are answered from fixtures.
pub struct Host<'a> {
    pub llm: &'a dyn Fn(usize) -> (u16, String),
    pub route_failures: usize,
    pub log: Vec<HttpRequest>,
}

impl Host<'_> {
    pub fn answer(&mut self, r: &HttpRequest) -> HttpResponse {
        self.log.push(r.clone());
        if r.credential == CredentialSlot::Llm {
            let n = self
                .log
                .iter()
                .filter(|x| x.credential == CredentialSlot::Llm)
                .count()
                - 1;
            let (status, content) = (self.llm)(n);
            let body = json!({ "choices": [{ "message": { "content": content } }] });
            return reply(r, status, serde_json::to_vec(&body).unwrap());
        }
        let u = &r.url;
        if u.contains("poiCategories") {
            reply(r, 200, fixture("poi_categories.json"))
        } else if u.contains("/geocode/") {
            reply(r, 200, fixture("geocode_berlin.json"))
        } else if u.contains("calculateRoute") {
            if self.route_failures > 0 {
                self.route_failures -= 1;
                return reply(r, 503, vec![]);
            }
            reply(r, 200, fixture("route_ams_ber.json"))
        } else if u.contains("searchAlongRoute") && u.contains("McDonald") {
            reply(r, 200, MCD.as_bytes().to_vec())
        } else if u.contains("searchAlongRoute") {
            reply(r, 200, fixture("search_chargers.json"))
        } else {
            panic!("unexpected request {u}")
        }
    }

    pub fn run(&mut self) -> Value {
        let (mut session, mut step) = engine().start(request()).unwrap();
        loop {
            match step {
                Step::Done { geojson } => return serde_json::from_str(&geojson).unwrap(),
                Step::NeedRequests { requests } => {
                    let resps = requests.iter().map(|r| self.answer(r)).collect();
                    step = session.step(resps).unwrap();
                }
            }
        }
    }
}

pub fn good_llm(_: usize) -> (u16, String) {
    (200, LLM_PLAN.to_string())
}

pub fn kinds(v: &Value, kind: &str) -> Vec<Value> {
    v["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["properties"]["kind"] == kind)
        .cloned()
        .collect()
}

impl Host<'_> {
    /// Drive any session to completion.
    pub fn drive(&mut self, mut session: Session, mut step: Step) -> Value {
        loop {
            match step {
                Step::Done { geojson } => return serde_json::from_str(&geojson).unwrap(),
                Step::NeedRequests { requests } => {
                    let resps = requests.iter().map(|r| self.answer(r)).collect();
                    step = session.step(resps).unwrap();
                }
            }
        }
    }
}

pub fn host(llm: &dyn Fn(usize) -> (u16, String)) -> Host<'_> {
    Host {
        llm,
        route_failures: 0,
        log: vec![],
    }
}
