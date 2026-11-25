//! Continuing a result with the end user's choices: no LLM, only the map calls the
//! change needs, and only places that were in the previous result.

mod common;

use common::*;
use serde_json::Value;
use waylingo::{
    Choices, CredentialSlot, Engine, EngineConfig, LlmConfig, LlmMode, LocationChoice,
    LocationRole, ProviderChoice, StopChoice, StopPick, TripResult, WaylingoError,
};

fn first_result() -> (String, Value) {
    let v = host(&good_llm).run();
    (serde_json::to_string(&v).unwrap(), v)
}

fn stop<'a>(v: &'a Value, id: &str, occurrence: u64) -> &'a Value {
    v["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| {
            f["properties"]["stop_request_id"] == id && f["properties"]["occurrence"] == occurrence
        })
        .unwrap_or_else(|| panic!("no feature for {id} #{occurrence}"))
}

fn place(id: &str) -> StopPick {
    StopPick::Place {
        provider_place_id: id.into(),
    }
}

fn choice(id: &str, occurrence: u32, pick: StopPick) -> StopChoice {
    StopChoice {
        stop_request_id: id.into(),
        occurrence,
        pick,
    }
}

#[test]
fn a_result_reads_back_unchanged() {
    let (text, original) = first_result();
    let doc = TripResult::parse(&text).unwrap();
    let again: Value = serde_json::to_value(&doc).unwrap();
    assert_eq!(again, original);
}

#[test]
fn ambiguous_places_list_their_candidates() {
    let (_, v) = first_result();
    let dest = kinds(&v, "destination");
    assert_eq!(dest.len(), 1);
    let p = &dest[0]["properties"];
    let candidates = p["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2, "both geocoding matches are offered");
    assert_eq!(p["chosen_id"], candidates[0]["id"]);
    assert_eq!(
        dest[0]["geometry"]["coordinates"],
        candidates[0]["position"]
    );
    assert!(p["along_m"].as_f64().unwrap() > 0.0);
    // The origin is the user's own position: no candidates to choose from.
    let origin = &kinds(&v, "origin")[0];
    assert_eq!(origin["properties"]["chosen_id"], Value::Null);
}

#[test]
fn stops_offer_alternatives_by_default() {
    let (_, v) = first_result();
    let food = stop(&v, "food", 0);
    let alts = food["properties"]["alternatives"].as_array().unwrap();
    assert_eq!(
        alts.len(),
        2,
        "top_n defaults to 3: the pick and two alternatives"
    );
    for a in alts {
        assert!(a["along_m"].is_number() && a["position"].is_array());
    }
}

#[test]
fn a_stop_choice_reroutes_with_one_map_call_and_no_llm() {
    let (text, before) = first_result();
    let alt = stop(&before, "food", 0)["properties"]["alternatives"][1]["provider_place_id"]
        .as_str()
        .unwrap()
        .to_string();
    let choices = Choices {
        stops: vec![
            choice("food", 0, place(&alt)),
            choice("charger", 1, StopPick::Skip),
        ],
        ..Choices::default()
    };
    let (session, step) = engine()
        .resume(&text, choices, Some("req-2".into()))
        .unwrap();
    let mut h = host(&good_llm);
    let after = h.drive(session, step);

    assert!(h.log.iter().all(|r| r.credential == CredentialSlot::Map));
    assert_eq!(h.log.len(), 1);
    assert!(h.log[0].url.contains("calculateRoute"));
    assert_eq!(after["usage"]["llm_calls"], 0);
    assert_eq!(after["usage"]["map_calls"], 1);
    assert_eq!(after["request_id"], "req-2");
    assert_eq!(after["plan_hash"], before["plan_hash"]);

    let food = &stop(&after, "food", 0)["properties"];
    assert_eq!(food["pick"]["provider_place_id"], alt.as_str());
    assert_eq!(food["user_choice"], "chosen");
    // The previous pick becomes the first alternative, so the user can switch back.
    assert_eq!(
        food["alternatives"][0]["provider_place_id"],
        stop(&before, "food", 0)["properties"]["pick"]["provider_place_id"]
    );
    let c1 = &stop(&after, "charger", 1)["properties"];
    assert_eq!(c1["kind"], "unresolved_stop");
    assert_eq!(c1["result"], "SKIPPED_BY_USER");
    assert_eq!(
        kinds(&after, "route")[0]["properties"]["includes_stops"],
        true
    );
}

#[test]
fn one_place_cannot_serve_two_stops() {
    // Offer stop #1's pick as an alternative of stop #0, then choose it there.
    let (_, mut v) = first_result();
    let taken = stop(&v, "charger", 1)["properties"]["pick"].clone();
    let i = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| {
            f["properties"]["stop_request_id"] == "charger" && f["properties"]["occurrence"] == 0
        })
        .unwrap();
    v["features"][i]["properties"]["alternatives"] = serde_json::json!([taken.clone()]);
    let choices = Choices {
        stops: vec![choice(
            "charger",
            0,
            place(taken["provider_place_id"].as_str().unwrap()),
        )],
        ..Choices::default()
    };
    assert!(matches!(
        engine().resume(&v.to_string(), choices, None),
        Err(WaylingoError::InvalidRequest(_))
    ));
}

#[test]
fn another_destination_searches_again_without_the_llm() {
    let (text, before) = first_result();
    let other = kinds(&before, "destination")[0]["properties"]["candidates"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let choices = Choices {
        locations: vec![LocationChoice {
            role: LocationRole::Destination,
            index: None,
            candidate_id: other.clone(),
        }],
        ..Choices::default()
    };
    let (session, step) = engine().resume(&text, choices, None).unwrap();
    let mut h = host(&good_llm);
    let after = h.drive(session, step);

    assert!(h.log.iter().all(|r| r.credential == CredentialSlot::Map));
    assert!(h.log.iter().any(|r| r.url.contains("searchAlongRoute")));
    assert!(
        !h.log.iter().any(|r| r.url.contains("/geocode/")),
        "no new geocoding"
    );
    assert_eq!(after["usage"]["llm_calls"], 0);
    let dest = &kinds(&after, "destination")[0];
    assert_eq!(dest["properties"]["chosen_id"], other.as_str());
    assert_eq!(
        dest["geometry"]["coordinates"],
        dest["properties"]["candidates"][1]["position"]
    );
}

#[test]
fn bad_continuations_are_errors() {
    let (text, _) = first_result();
    let e = engine();
    assert!(matches!(
        e.resume("{}", Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
    let unknown_candidate = Choices {
        locations: vec![LocationChoice {
            role: LocationRole::Destination,
            index: None,
            candidate_id: "made-up".into(),
        }],
        ..Choices::default()
    };
    assert!(matches!(
        e.resume(&text, unknown_candidate, None),
        Err(WaylingoError::UnknownChoice(_))
    ));
    let unknown_place = Choices {
        stops: vec![choice("charger", 0, place("not-an-option"))],
        ..Choices::default()
    };
    assert!(matches!(
        e.resume(&text, unknown_place, None),
        Err(WaylingoError::UnknownChoice(_))
    ));
    let unknown_stop = Choices {
        stops: vec![choice("dessert", 0, StopPick::Skip)],
        ..Choices::default()
    };
    assert!(matches!(
        e.resume(&text, unknown_stop, None),
        Err(WaylingoError::UnknownChoice(_))
    ));
    let here = Engine::new(EngineConfig::new(
        LlmConfig::new("https://llm.example/v1", "m", LlmMode::JsonSchema),
        ProviderChoice::Here { base_url: None },
    ));
    assert!(matches!(
        here.resume(&text, Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
}

fn run_resume(text: &str, choices: Choices) -> (Value, Vec<waylingo::HttpRequest>) {
    let (session, step) = engine().resume(text, choices, None).unwrap();
    let mut h = host(&good_llm);
    let v = h.drive(session, step);
    (v, h.log)
}

fn move_destination(v: &Value) -> Choices {
    Choices {
        locations: vec![LocationChoice {
            role: LocationRole::Destination,
            index: None,
            candidate_id: kinds(v, "destination")[0]["properties"]["candidates"][1]["id"]
                .as_str()
                .unwrap()
                .into(),
        }],
        ..Choices::default()
    }
}

fn stop_choices(stops: Vec<StopChoice>) -> Choices {
    Choices {
        stops,
        ..Choices::default()
    }
}

/// Every place id appears at most once across picks, alternatives and options.
fn assert_no_place_twice(v: &Value) {
    let mut seen = std::collections::HashSet::new();
    for f in v["features"].as_array().unwrap() {
        let p = &f["properties"];
        if p["kind"] != "stop" {
            continue;
        }
        let ids = std::iter::once(&p["pick"]).chain(p["alternatives"].as_array().unwrap());
        for x in ids {
            let id = x["provider_place_id"].as_str().unwrap().to_string();
            assert!(seen.insert(id.clone()), "{id} offered twice");
        }
    }
}

#[test]
fn no_place_is_offered_twice() {
    let (text, v) = first_result();
    assert_no_place_twice(&v);
    let (after, _) = run_resume(&text, move_destination(&v));
    assert_no_place_twice(&after);
}

#[test]
fn a_tampered_plan_is_rejected() {
    let (_, mut v) = first_result();
    v["plan"]["stop_requests"][0]["anchor"]["interval"]["distance_m"] = serde_json::json!(0.01);
    assert!(matches!(
        engine().resume(&v.to_string(), Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
}

#[test]
fn a_tampered_destination_geometry_is_ignored() {
    let (_, mut v) = first_result();
    let i = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["properties"]["kind"] == "destination")
        .unwrap();
    // Point the destination somewhere else without changing its chosen candidate.
    v["features"][i]["geometry"]["coordinates"] = serde_json::json!([2.35, 48.85]);
    let (after, log) = run_resume(&v.to_string(), stop_choices(vec![]));
    assert!(log[0].url.contains("52.520000,13.405000"), "{}", log[0].url);
    assert!(!log[0].url.contains("48.85"));
    assert_eq!(
        kinds(&after, "destination")[0]["geometry"]["coordinates"],
        serde_json::json!([13.405, 52.52])
    );
}

#[test]
fn signed_results_are_required_when_a_key_is_set() {
    let mut cfg = waylingo::EngineConfig::new(
        LlmConfig::new("https://llm.example/v1", "test-model", LlmMode::JsonSchema),
        ProviderChoice::TomTom { base_url: None },
    );
    cfg.signing_key = Some(waylingo::SigningKey::new(
        b"0123456789abcdef0123456789abcdef".to_vec(),
    ));
    let signer = Engine::new(cfg);
    let (session, step) = signer.start(request()).unwrap();
    let signed = host(&good_llm).drive(session, step);
    assert!(signed["signature"].as_str().unwrap().len() == 64);
    assert!(
        signer
            .resume(&signed.to_string(), Choices::default(), None)
            .is_ok()
    );

    let mut tampered = signed.clone();
    tampered["search"]["anchor_radius_m"] = serde_json::json!(50_000);
    assert!(matches!(
        signer.resume(&tampered.to_string(), Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
    let (unsigned, _) = first_result();
    assert!(matches!(
        signer.resume(&unsigned, Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
}

#[test]
fn a_skip_survives_a_destination_change() {
    let (text, v) = first_result();
    let (skipped, _) = run_resume(&text, stop_choices(vec![choice("food", 0, StopPick::Skip)]));
    let options = stop(&skipped, "food", 0)["properties"]["options"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(options, 3, "a skipped stop keeps what it offered");

    let (moved, log) = run_resume(&skipped.to_string(), move_destination(&v));
    assert_eq!(
        stop(&moved, "food", 0)["properties"]["result"],
        "SKIPPED_BY_USER"
    );
    assert!(
        !log.iter().any(|r| r.url.contains("McDonald")),
        "a skipped stop is not searched again"
    );
}

#[test]
fn a_skipped_stop_can_be_brought_back() {
    let (text, _) = first_result();
    let (skipped, _) = run_resume(&text, stop_choices(vec![choice("food", 0, StopPick::Skip)]));
    let back = stop(&skipped, "food", 0)["properties"]["options"][2]["provider_place_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (after, _) = run_resume(
        &skipped.to_string(),
        stop_choices(vec![choice("food", 0, place(&back))]),
    );
    let food = &stop(&after, "food", 0)["properties"];
    assert_eq!(food["kind"], "stop");
    assert_eq!(food["pick"]["provider_place_id"], back.as_str());
    assert_eq!(food["user_choice"], "chosen");
}

#[test]
fn a_stop_cannot_get_two_choices() {
    let (text, _) = first_result();
    let choices = stop_choices(vec![
        choice("food", 0, StopPick::Skip),
        choice("food", 0, place("synthetic-mcd-2")),
    ]);
    assert!(matches!(
        engine().resume(&text, choices, None),
        Err(WaylingoError::InvalidRequest(_))
    ));
}

#[test]
fn a_choice_not_found_after_a_re_search_is_marked_dropped() {
    let (_, mut v) = first_result();
    let i = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["properties"]["stop_request_id"] == "food")
        .unwrap();
    let mut ghost = v["features"][i]["properties"]["pick"].clone();
    ghost["provider_place_id"] = serde_json::json!("ghost-place");
    v["features"][i]["properties"]["alternatives"]
        .as_array_mut()
        .unwrap()
        .push(ghost);
    let mut choices = move_destination(&v);
    choices.stops = vec![choice("food", 0, place("ghost-place"))];
    let (after, _) = run_resume(&v.to_string(), choices);
    assert_eq!(
        stop(&after, "food", 0)["properties"]["user_choice"],
        "dropped"
    );
    assert!(after["warnings"].as_array().unwrap().iter().any(|w| {
        w.as_str()
            .unwrap()
            .contains("not found along the new route")
    }));
}

#[test]
fn a_failed_result_cannot_be_continued() {
    let llm = |_: usize| (401, String::new());
    let failed = host(&llm).run();
    assert!(matches!(
        engine().resume(&failed.to_string(), Choices::default(), None),
        Err(WaylingoError::IncompatibleResult(_))
    ));
}

#[test]
fn a_failed_reroute_is_reported_not_hidden() {
    let (text, _) = first_result();
    let (session, step) = engine()
        .resume(
            &text,
            stop_choices(vec![choice("food", 0, place("synthetic-mcd-2"))]),
            None,
        )
        .unwrap();
    let mut h = host(&good_llm);
    h.route_failures = 99;
    let v = h.drive(session, step);
    assert_eq!(v["status"], "failed");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("chosen places")
    );
}

#[test]
fn resuming_without_choices_is_stable() {
    let (text, _) = first_result();
    let (a, _) = run_resume(&text, Choices::default());
    let (b, _) = run_resume(&a.to_string(), Choices::default());
    assert_eq!(a["features"], b["features"]);
    assert_eq!(a["plan_hash"], b["plan_hash"]);
}

fn signing_engine() -> Engine {
    let mut cfg = waylingo::EngineConfig::new(
        LlmConfig::new("https://llm.example/v1", "test-model", LlmMode::JsonSchema),
        ProviderChoice::TomTom { base_url: None },
    );
    cfg.signing_key = Some(waylingo::SigningKey::new(
        b"0123456789abcdef0123456789abcdef".to_vec(),
    ));
    Engine::new(cfg)
}

#[test]
fn a_signed_result_resumes_with_the_route_line_stripped() {
    let e = signing_engine();
    let (session, step) = e.start(request()).unwrap();
    let mut v = host(&good_llm).drive(session, step);
    let i = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["properties"]["kind"] == "route")
        .unwrap();
    v["features"][i]["geometry"]["coordinates"] = serde_json::json!([]);
    let choices = stop_choices(vec![choice("food", 0, place("synthetic-mcd-2"))]);
    assert!(e.resume(&v.to_string(), choices, None).is_ok());
}

#[test]
fn signatures_survive_awkward_numbers() {
    // 123.456789 miles is a long-mantissa number of metres in the stored plan.
    let llm = |_: usize| {
        let plan = r#"{"destination":"Berlin","stops":[{"id":"charger","what":{"category":"ev_charging"},"anchor":{"kind":"every","amount":{"value":123.456789,"unit":"mi"}}}]}"#;
        (200, plan.to_string())
    };
    let e = signing_engine();
    let (session, step) = e.start(request()).unwrap();
    let text = host(&llm).drive(session, step).to_string();
    let doc = TripResult::parse(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        serde_json::from_str::<Value>(&text).unwrap(),
        "the document reads back exactly"
    );
    assert!(e.resume(&text, Choices::default(), None).is_ok());
}

#[test]
fn a_signed_result_resumes_with_the_route_geometry_nulled() {
    let e = signing_engine();
    let (session, step) = e.start(request()).unwrap();
    let mut v = host(&good_llm).drive(session, step);
    let i = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["properties"]["kind"] == "route")
        .unwrap();
    v["features"][i]["geometry"] = Value::Null;
    assert!(e.resume(&v.to_string(), Choices::default(), None).is_ok());
}
