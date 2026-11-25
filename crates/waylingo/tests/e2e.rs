//! End-to-end through the engine with a fake LLM and TomTom fixtures: query in,
//! GeoJSON out, no network.

mod common;

use common::*;
use serde_json::{Value, json};
use waylingo::{CredentialSlot, HttpRequest, HttpResponse, Step, TripRequest, WaylingoError};

#[test]
fn query_in_geojson_out() {
    let mut host = Host {
        llm: &good_llm,
        route_failures: 0,
        log: vec![],
    };
    let v = host.run();
    assert_eq!(v["type"], "FeatureCollection");
    assert_eq!(v["request_id"], "req-1");
    assert_eq!(v["provider"], "tomtom");
    assert!(v["attribution"].as_str().unwrap().contains("TomTom"));
    assert_eq!(v["plan"]["stop_requests"].as_array().unwrap().len(), 2);
    assert_eq!(v["plan_hash"].as_str().unwrap().len(), 64);

    let route = kinds(&v, "route");
    assert_eq!(route.len(), 1);
    assert_eq!(route[0]["geometry"]["type"], "LineString");
    assert_eq!(route[0]["properties"]["includes_stops"], true);
    // GeoJSON order is [lng, lat].
    assert_eq!(
        route[0]["geometry"]["coordinates"][0],
        json!([4.8952, 52.3702])
    );

    // Three charger marks share one fixture with two chargers: the same place is never
    // used twice, so the third mark is unresolved. The McDonald's is found.
    let stops = kinds(&v, "stop");
    let unresolved = kinds(&v, "unresolved_stop");
    assert_eq!(stops.len(), 3, "{v:#}");
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0]["properties"]["result"], "NO_CANDIDATES");
    assert_eq!(unresolved[0]["geometry"], Value::Null);
    assert_eq!(v["status"], "partial");
    let ids: Vec<&str> = stops
        .iter()
        .map(|s| {
            s["properties"]["pick"]["provider_place_id"]
                .as_str()
                .unwrap()
        })
        .collect();
    for want in [
        "synthetic-charger-1",
        "synthetic-charger-2",
        "synthetic-mcd-1",
    ] {
        assert!(ids.contains(&want), "{want} missing from {ids:?}");
    }
    for s in &stops {
        assert_eq!(s["geometry"]["type"], "Point");
        assert!(s["properties"]["pick"]["detour_s"].as_f64().unwrap() <= 600.0);
        assert!(s["properties"]["eta_s"].is_number());
    }
    let food = stops
        .iter()
        .find(|s| s["properties"]["stop_request_id"] == "food")
        .unwrap();
    assert_eq!(food["properties"]["pick"]["brand"], "McDonald's");
    // "15 min before arrival" is 900 s before the 23 400 s route end.
    assert_eq!(food["properties"]["anchor"]["cum_s"], 22_500.0);

    // 1 LLM call; categories + geocode + route + 4 searches + final route = 8 map calls.
    assert_eq!(v["usage"]["llm_calls"], 1);
    assert_eq!(v["usage"]["map_calls"], 8);
}

#[test]
fn provider_data_never_reaches_the_llm() {
    let mut host = Host {
        llm: &good_llm,
        route_failures: 0,
        log: vec![],
    };
    let _ = host.run();
    let mut seen_map = false;
    for r in &host.log {
        match r.credential {
            CredentialSlot::Map => seen_map = true,
            CredentialSlot::Llm => {
                assert!(!seen_map, "an LLM request was sent after a map request");
                let body = String::from_utf8(r.body.clone().unwrap_or_default()).unwrap();
                assert!(
                    !body.contains("WAYLINGO_CANARY"),
                    "provider data reached the LLM"
                );
                assert!(
                    !body.contains("52.3702") && !body.contains("4.8952"),
                    "location reached the LLM"
                );
            }
        }
        assert!(!r.url.contains("key="), "a request URL carried a key");
    }
    assert!(seen_map);
}

#[test]
fn same_inputs_give_identical_output() {
    let a = Host {
        llm: &good_llm,
        route_failures: 0,
        log: vec![],
    }
    .run();
    let b = Host {
        llm: &good_llm,
        route_failures: 0,
        log: vec![],
    }
    .run();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn llm_repair_then_success() {
    let llm = |n: usize| {
        if n == 0 {
            let bad = r#"{"destination":"Berlin","stops":[{"what":{"category":"charging"},"anchor":{"kind":"fraction","value":0.5}}]}"#;
            (200, bad.to_string())
        } else {
            (200, LLM_PLAN.to_string())
        }
    };
    let v = Host {
        llm: &llm,
        route_failures: 0,
        log: vec![],
    }
    .run();
    assert_eq!(v["usage"]["llm_calls"], 2);
    assert_ne!(v["status"], "failed");
}

#[test]
fn llm_failure_gives_failed_geojson_and_no_map_calls() {
    let llm = |_: usize| (401, String::new());
    let mut host = Host {
        llm: &llm,
        route_failures: 0,
        log: vec![],
    };
    let v = host.run();
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["code"], "LLM_AUTH");
    assert_eq!(v["features"].as_array().unwrap().len(), 0);
    assert_eq!(v["usage"]["map_calls"], 0);
    assert!(host.log.iter().all(|r| r.credential == CredentialSlot::Llm));
}

#[test]
fn route_503_is_retried() {
    let v = Host {
        llm: &good_llm,
        route_failures: 1,
        log: vec![],
    }
    .run();
    assert_ne!(v["status"], "failed");
    assert_eq!(v["usage"]["map_calls"], 9, "one extra call for the retry");
}

#[test]
fn route_keeps_failing_gives_failed_status() {
    let v = Host {
        llm: &good_llm,
        route_failures: 99,
        log: vec![],
    }
    .run();
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error"]["code"], "PROVIDER_ERROR");
}

#[test]
fn misuse_is_an_error() {
    let (mut s, step) = engine().start(request()).unwrap();
    let Step::NeedRequests { requests } = step else {
        panic!()
    };
    let bad = HttpResponse {
        id: "nope".into(),
        ..reply(&requests[0], 200, vec![])
    };
    assert!(matches!(
        s.step(vec![bad]),
        Err(WaylingoError::UnknownResponse(_))
    ));
    let empty = TripRequest {
        query: " ".into(),
        ..request()
    };
    assert!(matches!(
        engine().start(empty),
        Err(WaylingoError::InvalidRequest(_))
    ));
}

fn run_with(defaults: waylingo::Defaults) -> (Value, Vec<HttpRequest>) {
    let mut req = request();
    req.context.defaults = defaults;
    let mut host = Host {
        llm: &good_llm,
        route_failures: 0,
        log: vec![],
    };
    let (mut session, mut step) = engine().start(req).unwrap();
    loop {
        match step {
            Step::Done { geojson } => return (serde_json::from_str(&geojson).unwrap(), host.log),
            Step::NeedRequests { requests } => {
                let resps = requests.iter().map(|r| host.answer(r)).collect();
                step = session.step(resps).unwrap();
            }
        }
    }
}

#[test]
fn developer_settings_are_validated() {
    let bad = [
        waylingo::Defaults {
            anchor_radius_m: 10,
            ..Default::default()
        },
        waylingo::Defaults {
            search_limit: 0,
            ..Default::default()
        },
        waylingo::Defaults {
            anchor_radius_m: 5_000,
            search_span_m: Some(1_000),
            ..Default::default()
        },
        waylingo::Defaults {
            max_detour_s: 0,
            ..Default::default()
        },
        waylingo::Defaults {
            top_n: 9,
            ..Default::default()
        },
    ];
    for d in bad {
        let mut req = request();
        req.context.defaults = d;
        assert!(
            matches!(engine().start(req), Err(WaylingoError::InvalidRequest(_))),
            "{d:?} accepted"
        );
    }
}

#[test]
fn settings_reach_the_provider_and_the_output() {
    let d = waylingo::Defaults {
        anchor_radius_m: 2_000,
        search_limit: 50,
        search_span_m: Some(8_000),
        ..Default::default()
    };
    let (v, log) = run_with(d);
    assert_eq!(v["search"]["anchor_radius_m"], 2_000);
    assert_eq!(v["search"]["search_span_m"], 8_000);
    assert_eq!(
        v["search"]["search_limit"], 20,
        "capped at TomTom's maximum"
    );
    assert_eq!(v["search"]["search_limit_requested"], 50);
    let searches: Vec<&HttpRequest> = log
        .iter()
        .filter(|r| r.url.contains("searchAlongRoute"))
        .collect();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|r| r.url.contains("limit=20")));
    // Each search window is 2 x 8 km of route.
    for f in v["features"].as_array().unwrap() {
        let w = &f["properties"]["window"];
        if let (Some(a), Some(b)) = (w["start_m"].as_f64(), w["end_m"].as_f64()) {
            assert!(b - a <= 16_001.0, "window {a}..{b} wider than 2 x span");
        }
    }
}

#[test]
fn strict_radius_never_returns_far_places() {
    // The fixture places are tens of km from the anchors: lenient mode flags them,
    // strict mode reports the stops as not found.
    let (lenient, _) = run_with(waylingo::Defaults::default());
    let far = kinds(&lenient, "stop");
    assert!(far.iter().any(|s| s["properties"]["beyond_radius"] == true));

    let (strict, _) = run_with(waylingo::Defaults {
        strict_radius: true,
        ..Default::default()
    });
    assert_eq!(strict["search"]["strict_radius"], true);
    for s in kinds(&strict, "stop") {
        assert!(
            s["properties"]["pick"]["distance_from_anchor_m"]
                .as_f64()
                .unwrap()
                <= 5_000.0
        );
        assert_eq!(s["properties"]["beyond_radius"], false);
    }
    assert!(kinds(&strict, "unresolved_stop").len() >= kinds(&lenient, "unresolved_stop").len());
}
