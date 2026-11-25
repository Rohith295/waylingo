//! Live checks against the real TomTom API. Skipped unless run with `--ignored`.
//!
//! ```text
//! export WAYLINGO_TOMTOM_KEY=...        # your own key, never committed
//! cargo test -p waylingo-providers --test live_tomtom -- --ignored --nocapture
//! ```
//!
//! Output is pass/fail checks and counts only. Place names, addresses and
//! coordinates from TomTom are never printed, so provider data does not end up in
//! logs that an AI assistant might read. The key is never printed either.
//! About 8 TomTom requests per run (free tier: 2 500 search and 20 000 routing per month).

use std::time::Duration;

use waylingo_geo::{
    Amount, AnchorSpec, Direction, LatLng, Reference, Window, resolve, simplify_to_max_points,
};
use waylingo_http::{AuthPlacement, HttpRequest, HttpResponse};
use waylingo_plan::{Category, Connector, EvFilter, What};
use waylingo_providers::{ProviderAdapter, RouteQuery, RouteResult, SearchQuery, TomTom};

const AMS: LatLng = LatLng::new(52.3702, 4.8952);
const HANNOVER: LatLng = LatLng::new(52.3759, 9.7320);
const BER: LatLng = LatLng::new(52.5200, 13.4050);

struct Live {
    client: reqwest::blocking::Client,
    key: String,
    calls: u32,
}

impl Live {
    fn from_env() -> Option<Self> {
        let key = std::env::var("WAYLINGO_TOMTOM_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");
        Some(Self {
            client,
            key: key.trim().to_string(),
            calls: 0,
        })
    }

    /// Send like a host app would: add the key where the request says, nowhere else.
    fn send(&mut self, req: &HttpRequest) -> HttpResponse {
        std::thread::sleep(Duration::from_millis(250 + u64::from(req.not_before_ms))); // stay under 5 QPS
        let url = match &req.auth {
            AuthPlacement::QueryParam { name } => {
                let sep = if req.url.contains('?') { '&' } else { '?' };
                format!("{}{sep}{name}={}", req.url, self.key)
            }
            AuthPlacement::Header { .. } => req.url.clone(),
        };
        let mut b = match req.method.as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };
        for h in &req.headers {
            b = b.header(&h.name, &h.value);
        }
        if let AuthPlacement::Header { name, prefix } = &req.auth {
            b = b.header(name, format!("{prefix}{}", self.key));
        }
        if let Some(body) = &req.body {
            b = b.body(body.clone());
        }
        self.calls += 1;
        match b.send() {
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.bytes().map(|b| b.to_vec()).unwrap_or_default();
                HttpResponse {
                    id: req.id.clone(),
                    status,
                    headers: vec![],
                    body,
                    transport_error: None,
                }
            }
            // Print only the error kind: reqwest errors can include the URL, which holds the key.
            Err(e) => panic!(
                "request {} failed to send (timeout: {}, connect: {})",
                req.id,
                e.is_timeout(),
                e.is_connect()
            ),
        }
    }
}

fn check(ok: bool, what: &str) {
    println!("{} {what}", if ok { "PASS" } else { "FAIL" });
    assert!(ok, "{what}");
}

fn ev_charging(min_kw: f64) -> What {
    What {
        category: Some(Category::EvCharging),
        text: None,
        brand: None,
        ev: Some(EvFilter {
            min_power_kw: Some(min_kw),
            connector_types: vec![Connector::Ccs2],
        }),
    }
}

fn search_window(
    live: &mut Live,
    tt: &TomTom,
    rr: &RouteResult,
    spec: &AnchorSpec,
    half_m: f64,
    what: &What,
    label: &str,
) -> Vec<usize> {
    let anchors = resolve(&rr.route, spec).expect("anchor resolves");
    let mut counts = Vec::new();
    for (i, a) in anchors.iter().enumerate() {
        let w = Window::by_distance(&rr.route, a, half_m);
        let line = simplify_to_max_points(
            &rr.route.slice(w.start_m, w.end_m),
            tt.capabilities().max_search_points,
        );
        let q = SearchQuery {
            window_line: line,
            what: what.clone(),
            max_detour_s: 600,
            corridor_m: 5_000,
            limit: 20,
        };
        let resp = live.send(&tt.search_request(format!("{label}-{i}"), &q));
        check(
            resp.status == 200,
            &format!("{label} #{i}: search accepted (HTTP {})", resp.status),
        );
        let c = tt.parse_search(&resp).expect("search response parses");
        check(
            c.iter().all(|x| x.detour_s.is_some()),
            &format!("{label} #{i}: every result has native detourTime"),
        );
        check(
            c.iter().all(|x| x.detour_s.unwrap_or(f64::MAX) <= 600.0),
            &format!("{label} #{i}: every detour within maxDetourTime"),
        );
        println!(
            "     {label} #{i}: {} candidates in a {:.0} km window",
            c.len(),
            w.length_m() / 1000.0
        );
        counts.push(c.len());
    }
    counts
}

#[test]
#[ignore = "calls the live TomTom API; needs WAYLINGO_TOMTOM_KEY"]
fn live_tomtom_reference_trip() {
    let Some(mut live) = Live::from_env() else {
        println!("SKIP: WAYLINGO_TOMTOM_KEY is not set");
        return;
    };
    let mut tt = TomTom::default();

    // 1. Category ids from the live category list.
    for req in tt.setup_requests("setup-") {
        let resp = live.send(&req);
        check(
            resp.status == 200,
            &format!("category list fetched (HTTP {})", resp.status),
        );
        tt.apply_setup(&resp).expect("category list parses");
    }
    for (name, cat) in Category::ALL {
        let state = if tt.category_id(cat).is_some() {
            "matched"
        } else {
            "NOT matched (falls back to text query)"
        };
        println!("     category {name}: {state}");
    }
    check(
        tt.category_id(Category::EvCharging).is_some(),
        "EV charging category id found by name",
    );

    // 2. Geocode.
    let resp = live.send(&tt.geocode_request("geo-0".into(), "Berlin", Some(AMS)));
    check(
        resp.status == 200,
        &format!("geocode accepted (HTTP {})", resp.status),
    );
    let g = tt.parse_geocode(&resp).expect("geocode parses");
    check(!g.is_empty(), "geocode returned results");
    check(
        waylingo_geo::haversine_m(g[0].position, BER) < 30_000.0,
        "top geocode result is within 30 km of central Berlin",
    );

    // 3. Single-leg route.
    let q = RouteQuery {
        points: vec![AMS, BER],
        avoid: vec![],
        departure_time: None,
        electric: true,
    };
    let resp = live.send(&tt.route_request("route-0".into(), &q));
    check(
        resp.status == 200,
        &format!("route accepted (HTTP {})", resp.status),
    );
    let rr = tt
        .parse_route(&resp)
        .expect("route parses into a MeasuredRoute");
    let km = rr.route.total_m() / 1000.0;
    let h = rr.route.total_s() / 3600.0;
    check(
        (550.0..=750.0).contains(&km),
        &format!("route length plausible ({km:.0} km)"),
    );
    check(
        (5.0..=10.0).contains(&h),
        &format!("route duration plausible ({h:.1} h)"),
    );
    println!("     route: {} points", rr.route.coords().len());
    // Guidance checkpoints mean time is not spread evenly: speed should vary along the route.
    let (cum_s, cum_m) = (rr.route.cum_s(), rr.route.cum_m());
    let n = cum_s.len();
    let speed = |a: usize, b: usize| (cum_m[b] - cum_m[a]) / (cum_s[b] - cum_s[a]).max(1.0);
    let (first, middle) = (speed(0, n / 10), speed(n * 4 / 10, n * 6 / 10));
    check(
        (first - middle).abs() > 0.5,
        &format!(
            "speed varies along route (start {:.0} km/h, middle {:.0} km/h)",
            first * 3.6,
            middle * 3.6
        ),
    );

    // 4. Multi-leg route: checks that guidance pointIndex lines up across legs.
    let q = RouteQuery {
        points: vec![AMS, HANNOVER, BER],
        avoid: vec![],
        departure_time: None,
        electric: false,
    };
    let resp = live.send(&tt.route_request("route-1".into(), &q));
    check(
        resp.status == 200,
        &format!("multi-leg route accepted (HTTP {})", resp.status),
    );
    let rr2 = tt
        .parse_route(&resp)
        .expect("multi-leg route parses with valid checkpoints");
    check(rr2.leg_end_m.len() == 2, "two legs reported");
    let near = rr2
        .route
        .position_at_distance(rr2.leg_end_m[0])
        .expect("leg end on route");
    check(
        waylingo_geo::haversine_m(near.point, HANNOVER) < 5_000.0,
        "first leg ends within 5 km of the Hannover waypoint (pointIndex alignment)",
    );

    // 5. Along-route searches on the single-leg route.
    let every = AnchorSpec::Every {
        interval: Amount::DistanceM(200_000.0),
        min_tail: 30_000.0,
    };
    let marks = resolve(&rr.route, &every).expect("every-N resolves");
    check(
        marks.len() == 3,
        &format!("every 200 km gives 3 marks ({})", marks.len()),
    );
    // Search only the first mark to keep the call count low.
    let first_mark = AnchorSpec::Offset {
        from: Reference::Start,
        direction: Direction::After,
        amount: Amount::DistanceM(200_000.0),
    };
    let ev = search_window(
        &mut live,
        &tt,
        &rr,
        &first_mark,
        30_000.0,
        &ev_charging(150.0),
        "ev-150kW-ccs2",
    );
    check(
        ev.iter().sum::<usize>() > 0,
        "at least one 150 kW CCS2 charger near the 200 km mark",
    );

    let before_end = AnchorSpec::Offset {
        from: Reference::End,
        direction: Direction::Before,
        amount: Amount::DurationS(900.0),
    };
    let anchors = resolve(&rr.route, &before_end).expect("15 min before end resolves");
    let w = Window::by_time(&rr.route, &anchors[0], 120.0);
    let line = simplify_to_max_points(&rr.route.slice(w.start_m, w.end_m), 250);
    let mcd = What {
        category: Some(Category::FastFood),
        text: None,
        brand: Some("McDonald's".into()),
        ev: None,
    };
    let q = SearchQuery {
        window_line: line,
        what: mcd,
        max_detour_s: 600,
        corridor_m: 5_000,
        limit: 20,
    };
    let resp = live.send(&tt.search_request("mcd-0".into(), &q));
    check(
        resp.status == 200,
        &format!("brand search accepted (HTTP {})", resp.status),
    );
    let c = tt.parse_search(&resp).expect("brand search parses");
    let brand_hits = c
        .iter()
        .filter(|x| {
            x.brand
                .as_deref()
                .is_some_and(|b| b.to_lowercase().contains("mcdonald"))
        })
        .count();
    println!(
        "     brand search: {} results, {brand_hits} carry a McDonald's brand",
        c.len()
    );
    check(
        c.is_empty() || brand_hits > 0,
        "searching the name McDonald's finds McDonald's",
    );

    println!("DONE: {} live TomTom requests", live.calls);
}
