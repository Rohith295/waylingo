//! TomTom adapter against synthetic fixtures. The fixtures are hand-written in
//! TomTom's documented response shapes; they are not recorded TomTom data.

use waylingo_geo::{LatLng, simplify_to_max_points};
use waylingo_http::{AuthPlacement, CredentialSlot, HttpResponse, TransportError};
use waylingo_plan::{Avoid, Category, Connector, EvFilter, What};
use waylingo_providers::{
    DetourStrategy, ErrorKind, ProviderAdapter, ProviderKind, RouteQuery, SearchQuery, TomTom,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/tomtom/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn ok(id: &str, name: &str) -> HttpResponse {
    HttpResponse {
        id: id.into(),
        status: 200,
        headers: vec![],
        body: fixture(name),
        transport_error: None,
    }
}

fn status(s: u16) -> HttpResponse {
    HttpResponse {
        id: "x".into(),
        status: s,
        headers: vec![],
        body: b"{}".to_vec(),
        transport_error: None,
    }
}

const AMS: LatLng = LatLng::new(52.3702, 4.8952);
const BER: LatLng = LatLng::new(52.52, 13.405);

#[test]
fn identity_and_capabilities() {
    let t = TomTom::default();
    assert_eq!(t.kind(), ProviderKind::TomTom);
    assert_eq!(t.capabilities().detour, DetourStrategy::Native);
    assert!(t.attribution().contains("TomTom"));
}

#[test]
fn route_request_never_contains_a_key() {
    let t = TomTom::new(Some("https://proxy.example/tt/"));
    let q = RouteQuery {
        points: vec![AMS, BER],
        avoid: vec![Avoid::Tolls, Avoid::Ferries],
        departure_time: Some("2026-11-24T09:00:00+01:00".into()),
        electric: true,
    };
    let r = t.route_request("route-0".into(), &q);
    assert_eq!(r.method, "GET");
    assert!(r.url.starts_with(
        "https://proxy.example/tt/routing/1/calculateRoute/52.370200,4.895200:52.520000,13.405000/json?"
    ));
    for p in [
        "instructionsType=coded",
        "avoid=tollRoads",
        "avoid=ferries",
        "departAt=2026-11-24T09%3A00%3A00%2B01%3A00",
        "vehicleEngineType=electric",
    ] {
        assert!(r.url.contains(p), "missing {p} in {}", r.url);
    }
    assert!(!r.url.contains("key="));
    assert_eq!(r.credential, CredentialSlot::Map);
    assert_eq!(r.auth, AuthPlacement::QueryParam { name: "key".into() });
}

#[test]
fn parses_route_with_guidance_timing() {
    let t = TomTom::default();
    let rr = t.parse_route(&ok("route-0", "route_ams_ber.json")).unwrap();
    let r = &rr.route;
    assert!((r.total_m() - 654_000.0).abs() < 1e-6);
    assert!((r.total_s() - 23_400.0).abs() < 1e-6);
    assert_eq!(rr.leg_end_m, vec![654_000.0]);
    assert_eq!(rr.traffic_delay_s, Some(300.0));
    // Guidance says 2 h in is at 210 km (index 2), not 2/6.5 of 654 km.
    let p = r.position_at_time(7_200.0).unwrap();
    assert!((p.cum_m - 210_000.0).abs() < 1.0, "{p:?}");
    // The last 54 km take 3 600 s (slow city section).
    let p = r.position_at_time(23_400.0 - 900.0).unwrap();
    assert!((p.cum_m - 640_500.0).abs() < 1.0, "{p:?}");
}

#[test]
fn route_errors_are_classified() {
    let t = TomTom::default();
    assert_eq!(
        t.parse_route(&status(403)).unwrap_err().kind,
        ErrorKind::Auth
    );
    let e = t.parse_route(&status(429)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::RateLimited);
    assert!(e.retryable);
    assert_eq!(
        t.parse_route(&status(503)).unwrap_err().kind,
        ErrorKind::Upstream
    );
    let empty = HttpResponse {
        body: br#"{"routes":[]}"#.to_vec(),
        ..status(200)
    };
    assert_eq!(t.parse_route(&empty).unwrap_err().kind, ErrorKind::NoRoute);
    let garbage = HttpResponse {
        body: b"<html>".to_vec(),
        ..status(200)
    };
    assert_eq!(
        t.parse_route(&garbage).unwrap_err().kind,
        ErrorKind::BadResponse
    );
    let down = HttpResponse {
        transport_error: Some(TransportError::Timeout),
        ..status(0)
    };
    assert_eq!(t.parse_route(&down).unwrap_err().kind, ErrorKind::Transport);
}

#[test]
fn categories_are_learned_from_setup() {
    let mut t = TomTom::default();
    let setup = t.setup_requests("setup-");
    assert_eq!(setup.len(), 1);
    assert!(setup[0].url.contains("/search/2/poiCategories.json"));
    t.apply_setup(&ok("setup-categories", "poi_categories.json"))
        .unwrap();
    assert_eq!(t.category_id(Category::EvCharging), Some(9_000_001));
    assert_eq!(t.category_id(Category::FastFood), Some(9_000_004));
    assert_eq!(t.category_id(Category::Coffee), Some(9_000_005));
    assert_eq!(t.category_id(Category::Pharmacy), None);
}

#[test]
fn search_request_carries_filters_and_window() {
    let mut t = TomTom::default();
    t.apply_setup(&ok("s", "poi_categories.json")).unwrap();
    let window: Vec<LatLng> = (0..600)
        .map(|i| LatLng::new(52.3 + f64::from(i % 2) * 0.001, 6.0 + f64::from(i) * 0.003))
        .collect();
    let line = simplify_to_max_points(&window, t.capabilities().max_search_points);
    let q = SearchQuery {
        window_line: line.clone(),
        what: What {
            category: Some(Category::EvCharging),
            text: None,
            brand: None,
            ev: Some(EvFilter {
                min_power_kw: Some(150.0),
                connector_types: vec![Connector::Ccs2],
            }),
        },
        max_detour_s: 600,
        corridor_m: 5_000,
        limit: 50,
    };
    let r = t.search_request("search-0".into(), &q);
    assert_eq!(r.method, "POST");
    for p in [
        // Category known: the query is "*" (text + categorySet returns nothing live).
        "searchAlongRoute/%2A.json",
        "maxDetourTime=600",
        "limit=20",
        "sortBy=detourTime",
        "categorySet=9000001",
        "minPowerKW=150",
        "connectorSet=IEC62196Type2CCS",
    ] {
        assert!(r.url.contains(p), "missing {p} in {}", r.url);
    }
    let body: serde_json::Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
    let pts = body["route"]["points"].as_array().unwrap();
    assert_eq!(pts.len(), line.len());
    assert!(pts.len() <= 250);
    assert!(pts[0]["lat"].is_number() && pts[0]["lon"].is_number());
}

#[test]
fn brand_search_quotes_brand() {
    let t = TomTom::default();
    let q = SearchQuery {
        window_line: vec![AMS, BER],
        what: What {
            category: Some(Category::FastFood),
            brand: Some("McDonald's".into()),
            ..What::default()
        },
        max_detour_s: 9_999,
        corridor_m: 5_000,
        limit: 5,
    };
    let r = t.search_request("s".into(), &q);
    assert!(
        r.url.contains("searchAlongRoute/McDonald%27s.json"),
        "{}",
        r.url
    );
    assert!(
        !r.url.contains("brandSet"),
        "names are search text, not a chain filter: {}",
        r.url
    );
    assert!(
        r.url.contains("maxDetourTime=3600"),
        "clamped to TomTom's maximum"
    );
    assert!(
        !r.url.contains("categorySet"),
        "no ids learned yet, so no categorySet"
    );
}

#[test]
fn parses_search_with_native_detour_and_drops_invalid_points() {
    let t = TomTom::default();
    let c = t.parse_search(&ok("s", "search_chargers.json")).unwrap();
    assert_eq!(c.len(), 2, "result with lat 999 dropped");
    assert_eq!(c[0].provider_place_id, "synthetic-charger-1");
    assert_eq!(c[0].detour_s, Some(95.0));
    assert_eq!(c[0].detour_m, Some(1_200.0));
    assert!(c[0].brand.as_deref().unwrap().contains("WAYLINGO_CANARY_2"));
    assert!(c[1].address.is_none());
}

#[test]
fn parses_geocode() {
    let t = TomTom::default();
    let r = t.geocode_request("g".into(), "Berlin, Germany", Some(AMS));
    assert!(
        r.url.contains("/search/2/geocode/Berlin%2C%20Germany.json"),
        "{}",
        r.url
    );
    assert!(r.url.contains("lat=52.370200") && r.url.contains("lon=4.895200"));
    let g = t.parse_geocode(&ok("g", "geocode_berlin.json")).unwrap();
    assert_eq!(g.len(), 2);
    assert!((g[0].position.lat - 52.52).abs() < 1e-9);
    assert_eq!(g[0].score, Some(9.8));
}

#[test]
fn multi_leg_route_dedupes_joint_point() {
    let body = br#"{"routes":[{"summary":{"lengthInMeters":2000,"travelTimeInSeconds":200},
        "legs":[{"summary":{"lengthInMeters":1000,"travelTimeInSeconds":100},"points":[{"latitude":0,"longitude":0},{"latitude":0,"longitude":0.009}]},
                {"summary":{"lengthInMeters":1000,"travelTimeInSeconds":100},"points":[{"latitude":0,"longitude":0.009},{"latitude":0,"longitude":0.018}]}]}]}"#;
    let rr = TomTom::default()
        .parse_route(&HttpResponse {
            body: body.to_vec(),
            ..status(200)
        })
        .unwrap();
    assert_eq!(rr.route.coords().len(), 3);
    assert_eq!(rr.leg_end_m, vec![1_000.0, 2_000.0]);
}
