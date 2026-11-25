//! HERE adapter against synthetic responses in HERE's documented shapes. No HERE
//! data is stored in this repository (HERE terms forbid exposing it to open source).

use serde_json::json;
use waylingo_geo::{LatLng, polyline::encode_here};
use waylingo_http::{AuthPlacement, CredentialSlot, HttpResponse};
use waylingo_plan::{Avoid, Category, EvFilter, What};
use waylingo_providers::{
    DetourStrategy, ErrorKind, Here, ProviderAdapter, RouteQuery, SearchQuery,
};

fn ok(body: &serde_json::Value) -> HttpResponse {
    HttpResponse {
        id: "x".into(),
        status: 200,
        headers: vec![],
        body: serde_json::to_vec(body).unwrap(),
        transport_error: None,
    }
}

fn line(lat: f64, lng0: f64, n: usize) -> Vec<LatLng> {
    (0..n)
        .map(|i| LatLng::new(lat, lng0 + i as f64 * 0.01))
        .collect()
}

#[test]
fn identity_and_capabilities() {
    let h = Here::default();
    assert_eq!(h.capabilities().detour, DetourStrategy::Matrix);
    assert!(h.attribution().contains("HERE"));
    assert!(h.setup_requests("").is_empty());
}

#[test]
fn route_request_encodes_brackets_and_never_holds_a_key() {
    let q = RouteQuery {
        points: vec![
            LatLng::new(52.37, 4.89),
            LatLng::new(52.37, 9.73),
            LatLng::new(52.52, 13.40),
        ],
        avoid: vec![Avoid::Tolls, Avoid::Ferries],
        departure_time: Some("2026-11-24T09:00:00+01:00".into()),
        electric: false,
    };
    let r = Here::default().route_request("r".into(), &q);
    assert!(r.url.starts_with("https://router.hereapi.com/v8/routes?"));
    for p in [
        "transportMode=car",
        "return=polyline%2Csummary%2Cactions",
        "origin=52.370000%2C4.890000",
        "via=52.370000%2C9.730000",
        "destination=52.520000%2C13.400000",
        "avoid%5Bfeatures%5D=tollRoad%2Cferry",
    ] {
        assert!(r.url.contains(p), "missing {p} in {}", r.url);
    }
    assert!(!r.url.contains('['), "raw brackets are rejected by HERE");
    assert!(!r.url.to_lowercase().contains("apikey"));
    assert_eq!(r.credential, CredentialSlot::Map);
    assert_eq!(
        r.auth,
        AuthPlacement::QueryParam {
            name: "apiKey".into()
        }
    );
}

#[test]
fn parses_two_section_route_with_action_timing() {
    // Leg 1: 11 points, 10 km; first 5 km fast (150 s), last 5 km slow (450 s).
    // Leg 2: 11 points sharing the joint point, 10 km in 600 s.
    let a = line(52.0, 5.0, 11);
    let b = line(52.0, 5.1, 11);
    let body = json!({ "routes": [{ "sections": [
        { "polyline": encode_here(&a).unwrap(), "summary": { "length": 10_000, "duration": 600, "baseDuration": 560 },
          "actions": [ { "offset": 0, "length": 5_000, "duration": 150 }, { "offset": 5, "length": 5_000, "duration": 450 }, { "offset": 10, "length": 0, "duration": 0 } ] },
        { "polyline": encode_here(&b).unwrap(), "summary": { "length": 10_000, "duration": 600, "baseDuration": 600 },
          "actions": [ { "offset": 0, "length": 10_000, "duration": 600 } ] }
    ]}]});
    let rr = Here::default().parse_route(&ok(&body)).unwrap();
    assert_eq!(rr.route.coords().len(), 21, "joint point not duplicated");
    assert_eq!(rr.leg_end_m, vec![10_000.0, 20_000.0]);
    assert!((rr.route.total_m() - 20_000.0).abs() < 1e-6);
    assert!((rr.route.total_s() - 1_200.0).abs() < 1e-6);
    assert_eq!(rr.traffic_delay_s, Some(40.0));
    // 150 s in is the 5 km point, not 1/8 of the way.
    let p = rr.route.position_at_time(150.0).unwrap();
    assert!((p.cum_m - 5_000.0).abs() < 1.0, "{p:?}");
    // The leg boundary sits exactly at 10 km / 600 s.
    let p = rr.route.position_at_distance(10_000.0).unwrap();
    assert!((p.cum_s - 600.0).abs() < 1e-6);
}

#[test]
fn route_errors() {
    let h = Here::default();
    let empty = ok(&json!({ "routes": [] }));
    assert_eq!(h.parse_route(&empty).unwrap_err().kind, ErrorKind::NoRoute);
    let mut bad =
        ok(&json!({ "title": "Malformed request", "cause": "Invalid value for parameter" }));
    bad.status = 400;
    let e = h.parse_route(&bad).unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadRequest);
    assert!(e.message.contains("Malformed request"), "{}", e.message);
}

#[test]
fn category_search_uses_browse_with_ev_power() {
    let q = SearchQuery {
        window_line: line(52.3, 7.0, 30),
        what: What {
            category: Some(Category::EvCharging),
            ev: Some(EvFilter {
                min_power_kw: Some(150.0),
                connector_types: vec![],
            }),
            ..What::default()
        },
        max_detour_s: 600,
        limit: 50,
        corridor_m: 5_000,
    };
    let r = Here::default().search_request("s".into(), &q);
    assert!(
        r.url
            .starts_with("https://browse.search.hereapi.com/v1/browse?"),
        "{}",
        r.url
    );
    for p in [
        "categories=700-7600-0322",
        "evStation%5BminPower%5D=150",
        "show=ev",
        "limit=50",
        "%3Bw%3D5000",
    ] {
        assert!(r.url.contains(p), "missing {p} in {}", r.url);
    }
}

#[test]
fn brand_or_text_search_uses_discover() {
    let q = SearchQuery {
        window_line: line(52.3, 7.0, 30),
        what: What {
            category: Some(Category::FastFood),
            brand: Some("McDonald's".into()),
            ..What::default()
        },
        max_detour_s: 600,
        limit: 20,
        corridor_m: 3_000,
    };
    let r = Here::default().search_request("s".into(), &q);
    assert!(
        r.url
            .starts_with("https://discover.search.hereapi.com/v1/discover?"),
        "{}",
        r.url
    );
    assert!(r.url.contains("q=McDonald%27s"));
}

#[test]
fn parses_search_items() {
    let body = json!({ "items": [
        { "id": "here:pds:place:1", "title": "WAYLINGO_CANARY_1 Charger", "position": { "lat": 52.3, "lng": 7.1 },
          "address": { "label": "WAYLINGO_CANARY_2" }, "excursionDistance": 1800, "chains": [{ "id": "1", "name": "WAYLINGO_CANARY_3" }] },
        { "id": "here:pds:place:2", "title": "No position" },
        { "title": "Bad position", "position": { "lat": 99, "lng": 7.2 } }
    ]});
    let c = Here::default().parse_search(&ok(&body)).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].provider_place_id, "here:pds:place:1");
    assert_eq!(c[0].detour_s, None, "filled later by the matrix step");
    assert_eq!(c[0].detour_m, Some(1_800.0));
    assert!(c[0].brand.as_deref().unwrap().contains("CANARY_3"));
}

#[test]
fn matrix_request_and_parse() {
    let h = Here::default();
    let o = vec![LatLng::new(52.0, 5.0), LatLng::new(52.0, 5.1)];
    let d = vec![LatLng::new(52.0, 5.1), LatLng::new(52.0, 5.2)];
    let r = h.matrix_request("m".into(), &o, &d).unwrap();
    assert_eq!(r.method, "POST");
    assert!(r.url.contains("async=false"));
    let b: serde_json::Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
    assert_eq!(b["origins"].as_array().unwrap().len(), 2);
    assert_eq!(b["regionDefinition"]["type"], "world");
    let resp = ok(
        &json!({ "matrix": { "numOrigins": 2, "numDestinations": 2, "travelTimes": [60, 120, 0, 70], "errorCodes": [0, 0, 3, 0] } }),
    );
    let t = h.parse_matrix(&resp, 2, 2).unwrap();
    assert_eq!(t, vec![Some(60.0), Some(120.0), None, Some(70.0)]);
    let short = ok(&json!({ "matrix": { "travelTimes": [1, 2, 3] } }));
    assert!(h.parse_matrix(&short, 2, 2).is_err());
}
