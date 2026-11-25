//! Live contract tests: the real map and LLM APIs, through the public API that
//! Python, Swift and Kotlin use. They catch provider-side changes (response format,
//! retired models, auth changes) that the offline fixtures cannot. Skipped unless
//! run with `--ignored`; each case is skipped when its keys are not set.
//!
//! ```text
//! TOMTOM_API_KEY=... HERE_API_KEY=... ANTHROPIC_API_KEY=... \
//! OPENAI_API_KEY=... OPENAI_MODEL=... GEMINI_API_KEY=... GEMINI_MODEL=... \
//!   cargo test -p waylingo-ffi --test live -- --ignored --test-threads 1 --nocapture
//! ```
//!
//! `WAYLINGO_LIVE_REQUIRE` (comma-separated: tomtom, here, anthropic, openai,
//! gemini) turns a missing key for those services into a failure instead of a
//! skip. Run by hand; the nightly job covers the map providers only.
//!
//! Output is statuses, codes and counts only. Place names, addresses and
//! coordinates from the providers are never printed, and keys never are.

use std::sync::Arc;

use waylingo_ffi::{
    Choices, ErrorCode, LatLng, LlmBackend, MapProvider, SearchSettings, StopChoice, StopPick,
    TripInput, TripOutput, TripStatus, UserChoice, Waylingo, WaylingoConfig,
};

const AMSTERDAM: LatLng = LatLng {
    lat: 52.3702,
    lng: 4.8952,
};

/// A value from the environment, or `None` (skip) unless the service is required.
fn env(var: &str, service: &str) -> Option<String> {
    let v = std::env::var(var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if v.is_none() {
        let required = std::env::var("WAYLINGO_LIVE_REQUIRE").unwrap_or_default();
        assert!(
            !required.split(',').any(|s| s.trim() == service),
            "{var} is not set but {service} is required"
        );
        println!("SKIP {service}: {var} is not set");
    }
    v
}

fn map(name: &str) -> Option<(MapProvider, String)> {
    match name {
        "tomtom" => env("TOMTOM_API_KEY", name).map(|k| (MapProvider::Tomtom, k)),
        "here" => env("HERE_API_KEY", name).map(|k| (MapProvider::Here, k)),
        _ => unreachable!("unknown map provider {name}"),
    }
}

fn llm(name: &str) -> Option<(LlmBackend, String)> {
    let backend = match name {
        "anthropic" => LlmBackend::Anthropic {
            model: std::env::var("ANTHROPIC_MODEL").ok(),
            base_url: None,
        },
        "openai" => LlmBackend::OpenAi {
            model: env("OPENAI_MODEL", name)?,
            base_url: None,
        },
        "gemini" => LlmBackend::Gemini {
            model: env("GEMINI_MODEL", name)?,
            base_url: None,
        },
        _ => unreachable!("unknown LLM {name}"),
    };
    let var = format!("{}_API_KEY", name.to_uppercase());
    env(&var, name).map(|k| (backend, k))
}

fn app(map: MapProvider, map_key: String, llm: LlmBackend, llm_key: String) -> Arc<Waylingo> {
    let config = WaylingoConfig {
        map_provider: map,
        llm,
        map_base_url: None,
        settings: Some(SearchSettings {
            anchor_radius_m: None,
            search_span_m: None,
            search_limit: None,
            max_detour_s: None,
            top_n: Some(3),
            strict_radius: None,
        }),
        max_map_calls: None,
        map_requests_per_second: None,
        timeout_ms: Some(120_000),
    };
    Waylingo::new(config, map_key, Some(llm_key), None).expect("valid config")
}

fn trip(query: &str) -> TripInput {
    TripInput {
        query: query.into(),
        current_location: Some(AMSTERDAM),
        heading_deg: None,
        locale: Some("en-GB".into()),
        saved_places: vec![],
        active_destination: None,
        ev: None,
        avoid: vec![],
        departure_time: None,
        timezone: None,
        request_id: None,
    }
}

fn summary(label: &str, out: &TripOutput) {
    println!(
        "{label}: status {:?}, error {:?}, stops {}, unresolved {}, llm calls {}, map calls {}, warnings {}",
        out.status,
        out.error_code,
        out.stops.len(),
        out.unresolved.len(),
        out.llm_calls,
        out.map_calls,
        out.warnings.len()
    );
    // Engine and validation messages only; provider data never goes into them.
    if let Some(m) = &out.error_message {
        println!("{label}: message {m}");
    }
}

/// The result is a GeoJSON FeatureCollection with a route line.
fn check_geojson(out: &TripOutput) {
    let v: serde_json::Value = serde_json::from_str(&out.geojson).expect("geojson is JSON");
    assert_eq!(v["type"], "FeatureCollection");
    let features = v["features"].as_array().expect("features array");
    assert!(
        features
            .iter()
            .any(|f| f["properties"]["kind"] == "route" && f["geometry"]["type"] == "LineString"),
        "a route feature with a LineString"
    );
}

/// Plan a trip with one stop, then pick an alternative: covers the LLM parse,
/// geocode, routing, along-route search, detour ranking, and reroute.
fn plan_and_reroute(map_name: &str, llm_name: &str) {
    let (Some((mp, map_key)), Some((backend, llm_key))) = (map(map_name), llm(llm_name)) else {
        return;
    };
    let label = format!("{map_name}+{llm_name}");
    let app = app(mp, map_key, backend, llm_key);

    let out = app
        .plan_blocking(trip("Drive to Utrecht, coffee halfway"))
        .expect("plan returns a result");
    summary(&format!("{label} plan"), &out);
    assert_ne!(
        out.status,
        TripStatus::Failed,
        "plan failed: {:?}",
        out.error_code
    );
    assert!(out.llm_calls >= 1, "the LLM was called");
    assert!(
        !out.attribution.is_empty(),
        "provider attribution is present"
    );
    let km = out.route_m.expect("route length") / 1000.0;
    assert!(
        (25.0..=90.0).contains(&km),
        "route length plausible ({km:.0} km)"
    );
    assert_eq!(
        out.stops.len() + out.unresolved.len(),
        1,
        "one stop requested"
    );
    check_geojson(&out);

    let Some(stop) = out.stops.iter().find(|s| !s.alternatives.is_empty()) else {
        println!("{label}: no alternatives returned; reroute not exercised");
        return;
    };
    let alt = stop.alternatives[0].provider_place_id.clone();
    let choices = Choices {
        locations: vec![],
        stops: vec![StopChoice {
            stop_request_id: stop.stop_request_id.clone(),
            occurrence: stop.occurrence,
            pick: StopPick::Place {
                provider_place_id: alt.clone(),
            },
        }],
    };
    let again = app
        .reroute_blocking(out.geojson.clone(), choices, None)
        .expect("reroute returns a result");
    summary(&format!("{label} reroute"), &again);
    assert_ne!(
        again.status,
        TripStatus::Failed,
        "reroute failed: {:?}",
        again.error_code
    );
    assert_eq!(again.llm_calls, 0, "reroute does not call the LLM");
    let chosen = again
        .stops
        .iter()
        .find(|s| s.stop_request_id == stop.stop_request_id)
        .expect("the stop is still there");
    assert_eq!(
        chosen.pick.provider_place_id, alt,
        "the chosen place is the pick"
    );
    assert_eq!(chosen.user_choice, Some(UserChoice::Chosen));
    check_geojson(&again);
}

/// Text that is not a trip ends with NOT_UNDERSTOOD and no map calls.
fn not_a_trip(llm_name: &str) {
    let (Some((mp, map_key)), Some((backend, llm_key))) = (map("tomtom"), llm(llm_name)) else {
        return;
    };
    let out = app(mp, map_key, backend, llm_key)
        .plan_blocking(trip("What is the capital of Peru?"))
        .expect("plan returns a result");
    summary(&format!("{llm_name} not-a-trip"), &out);
    assert_eq!(out.status, TripStatus::Failed);
    assert_eq!(out.error_code, Some(ErrorCode::NotUnderstood));
    assert_eq!(out.map_calls, 0, "nothing was searched");
}

/// A wrong key is reported as an auth error, not as something vaguer.
fn wrong_key(map_name: &str, llm_name: &str, break_map: bool) {
    let (Some((mp, map_key)), Some((backend, llm_key))) = (map(map_name), llm(llm_name)) else {
        return;
    };
    let bad = "not-a-real-key".to_string();
    let (map_key, llm_key, want, what) = if break_map {
        (bad, llm_key, ErrorCode::ProviderAuth, "map")
    } else {
        (map_key, bad, ErrorCode::LlmAuth, "LLM")
    };
    let out = app(mp, map_key, backend, llm_key)
        .plan_blocking(trip("Drive to Utrecht, coffee halfway"))
        .expect("plan returns a result");
    summary(&format!("{map_name}+{llm_name} wrong {what} key"), &out);
    assert_eq!(out.status, TripStatus::Failed);
    assert_eq!(out.error_code, Some(want));
}

#[test]
#[ignore = "live: TomTom + Anthropic"]
fn tomtom_anthropic() {
    plan_and_reroute("tomtom", "anthropic");
}

#[test]
#[ignore = "live: HERE + Anthropic"]
fn here_anthropic() {
    plan_and_reroute("here", "anthropic");
}

#[test]
#[ignore = "live: TomTom + OpenAI"]
fn tomtom_openai() {
    plan_and_reroute("tomtom", "openai");
}

#[test]
#[ignore = "live: TomTom + Gemini"]
fn tomtom_gemini() {
    plan_and_reroute("tomtom", "gemini");
}

#[test]
#[ignore = "live: Anthropic, OpenAI, Gemini"]
fn not_a_trip_is_reported() {
    for name in ["anthropic", "openai", "gemini"] {
        not_a_trip(name);
    }
}

#[test]
#[ignore = "live: TomTom, HERE, Anthropic"]
fn wrong_keys_are_auth_errors() {
    wrong_key("tomtom", "anthropic", true);
    wrong_key("here", "anthropic", true);
    wrong_key("tomtom", "anthropic", false);
}

/// A destination with no stops: one LLM call, geocode and route only.
fn destination_only(map_name: &str, llm_name: &str) {
    let (Some((mp, map_key)), Some((backend, llm_key))) = (map(map_name), llm(llm_name)) else {
        return;
    };
    let out = app(mp, map_key, backend, llm_key)
        .plan_blocking(trip("Drive to Utrecht"))
        .expect("plan returns a result");
    summary(&format!("{map_name}+{llm_name} destination only"), &out);
    assert_eq!(out.status, TripStatus::Complete, "{:?}", out.error_code);
    assert!(out.stops.is_empty() && out.unresolved.is_empty());
    assert_eq!(out.llm_calls, 1, "no repair calls");
    check_geojson(&out);
}

#[test]
#[ignore = "live: TomTom, HERE, Anthropic"]
fn destination_only_trips() {
    destination_only("tomtom", "anthropic");
    destination_only("here", "anthropic");
}
