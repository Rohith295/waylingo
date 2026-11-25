//! Live scenario suite: every supported intent, run against the real LLM and map
//! provider, with expected results checked automatically.
//!
//! ```text
//! cargo run -q -p waylingo-reqwest --example scenarios            # all scenarios
//! cargo run -q -p waylingo-reqwest --example scenarios -- every   # names containing "every"
//! ```
//!
//! Uses the same `.env` settings as the `plan` example. Prints PASS/FAIL per check,
//! counts and distances only: no place names or addresses from the provider. Exits
//! non-zero if any check fails. Roughly 1 LLM call and 5-9 map calls per scenario.

use std::collections::HashSet;
use std::process::ExitCode;

use serde_json::{Value, json};
use waylingo::{
    Connector, Coords, Defaults, Engine, EngineConfig, EvFilter, SavedPlace, TripContext,
    TripRequest, Vehicle,
};
use waylingo_reqwest::{Keys, drive};

#[path = "common/settings.rs"]
mod settings;
use settings::{llm_config, load_dotenv, map_provider, setting};

const AMSTERDAM: (f64, f64) = (52.3702, 4.8952);
const UTRECHT: (f64, f64) = (52.0907, 5.1214);
const BERLIN: (f64, f64) = (52.5200, 13.4050);
const RADIUS_M: f64 = 5_000.0;

type Check = (&'static str, Box<dyn Fn(&Value) -> bool>);

struct Scenario {
    name: &'static str,
    query: &'static str,
    at: (f64, f64),
    ev: bool,
    saved: Vec<(&'static str, (f64, f64))>,
    active_destination: Option<(f64, f64)>,
    defaults: Defaults,
    expect_invalid: bool,
    checks: Vec<Check>,
}

impl Scenario {
    fn new(name: &'static str, query: &'static str) -> Self {
        Self {
            name,
            query,
            at: AMSTERDAM,
            ev: false,
            saved: vec![],
            active_destination: None,
            defaults: Defaults::default(),
            expect_invalid: false,
            checks: vec![],
        }
    }
    fn at(mut self, p: (f64, f64)) -> Self {
        self.at = p;
        self
    }
    fn ev(mut self) -> Self {
        self.ev = true;
        self
    }
    fn saved(mut self, name: &'static str, p: (f64, f64)) -> Self {
        self.saved.push((name, p));
        self
    }
    fn driving_to(mut self, p: (f64, f64)) -> Self {
        self.active_destination = Some(p);
        self
    }
    fn settings(mut self, d: Defaults) -> Self {
        self.defaults = d;
        self
    }
    fn expect_invalid(mut self) -> Self {
        self.expect_invalid = true;
        self
    }
    fn check(mut self, what: &'static str, f: impl Fn(&Value) -> bool + 'static) -> Self {
        self.checks.push((what, Box::new(f)));
        self
    }
}

// ---------- helpers over the output GeoJSON ----------

fn status(v: &Value) -> &str {
    v["status"].as_str().unwrap_or("")
}
fn features<'a>(v: &'a Value, kind: &str) -> Vec<&'a Value> {
    v["features"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|f| f["properties"]["kind"] == kind)
                .collect()
        })
        .unwrap_or_default()
}
fn stop_requests(v: &Value) -> Vec<&Value> {
    v["plan"]["stop_requests"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}
/// The first stop request whose category is one of `cats`, or whose text/brand contains `word`.
fn find_sr<'a>(v: &'a Value, cats: &[&str], word: &str) -> Option<&'a Value> {
    stop_requests(v).into_iter().find(|s| {
        let w = &s["what"];
        let cat = w["category"].as_str().unwrap_or("");
        let txt = format!(
            "{} {}",
            w["text"].as_str().unwrap_or(""),
            w["brand"].as_str().unwrap_or("")
        )
        .to_lowercase();
        cats.contains(&cat) || (!word.is_empty() && txt.contains(word))
    })
}
fn anchor_kind(sr: &Value) -> &str {
    sr["anchor"]["kind"].as_str().unwrap_or("")
}
/// ("m", metres) or ("s", seconds) of an offset amount or every interval.
fn amount(sr: &Value) -> Option<(&'static str, f64)> {
    let a = &sr["anchor"];
    let x = if a["kind"] == "every" {
        &a["interval"]
    } else {
        &a["amount"]
    };
    if let Some(m) = x["distance_m"].as_f64() {
        Some(("m", m))
    } else {
        x["duration_s"].as_f64().map(|s| ("s", s))
    }
}
fn amount_is(sr: &Value, unit: &str, value: f64) -> bool {
    amount(sr).is_some_and(|a| a.0 == unit && near(a.1, value, 1.0))
}
fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
fn route_total_m(v: &Value) -> f64 {
    features(v, "route")
        .first()
        .and_then(|r| r["properties"]["total_m"].as_f64())
        .unwrap_or(0.0)
}
fn results_for<'a>(v: &'a Value, stop_id: &str) -> Vec<&'a Value> {
    v["features"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|f| f["properties"]["stop_request_id"] == stop_id)
                .collect()
        })
        .unwrap_or_default()
}
fn id(sr: &Value) -> String {
    sr["id"].as_str().unwrap_or("").to_string()
}
fn anchor_values(v: &Value, sr: &Value, key: &str) -> Vec<f64> {
    results_for(v, &id(sr))
        .iter()
        .filter_map(|f| f["properties"]["anchor"][key].as_f64())
        .collect()
}

// ---------- shared checks (added to every scenario that succeeds) ----------

fn ok_status(v: &Value) -> bool {
    matches!(status(v), "complete" | "partial")
}
fn detours_within_limit(v: &Value) -> bool {
    let limit = v["plan"]["max_detour_s"].as_f64().unwrap_or(600.0);
    features(v, "stop").iter().all(|s| {
        s["properties"]["pick"]["detour_s"]
            .as_f64()
            .is_none_or(|d| d <= limit)
    })
}
/// Every found stop is within the radius of its anchor, or is explicitly flagged.
fn within_radius_or_flagged(v: &Value) -> bool {
    features(v, "stop").iter().all(|s| {
        let p = &s["properties"];
        let d = p["distance_from_anchor_m"].as_f64().unwrap_or(f64::MAX);
        (d <= RADIUS_M && p["beyond_radius"] == false)
            || (d > RADIUS_M && p["beyond_radius"] == true)
    })
}
fn budget_ok(v: &Value) -> bool {
    v["usage"]["map_calls"].as_u64().unwrap_or(99) <= 25
        && v["usage"]["llm_calls"].as_u64().unwrap_or(99) <= 3
}
fn at_least_stops(n: usize) -> impl Fn(&Value) -> bool {
    move |v| features(v, "stop").len() >= n
}

#[allow(clippy::too_many_lines)]
fn scenarios() -> Vec<Scenario> {
    vec![
        // ---- every N ----
        Scenario::new(
            "every-distance-km",
            "Drive to Berlin, I need a fast charger every 200 km",
        )
        .ev()
        .check("status complete or partial", ok_status)
        .check("anchor is every 200 000 m", |v| {
            find_sr(v, &["ev_charging"], "charg")
                .is_some_and(|s| anchor_kind(s) == "every" && amount_is(s, "m", 200_000.0))
        })
        .check("min power 150 kW", |v| {
            find_sr(v, &["ev_charging"], "charg")
                .is_some_and(|s| s["what"]["ev"]["min_power_kw"].as_f64() == Some(150.0))
        })
        .check("marks at 200/400/600 km", |v| {
            find_sr(v, &["ev_charging"], "charg").is_some_and(|s| {
                let m = anchor_values(v, s, "cum_m");
                m.len() == 3
                    && m.iter()
                        .enumerate()
                        .all(|(i, x)| near(*x, 200_000.0 * (i as f64 + 1.0), 1.0))
            })
        })
        .check("at least 2 chargers found", at_least_stops(2)),
        Scenario::new(
            "every-time-hours",
            "Drive to Munich, I want a coffee break every 2 hours",
        )
        .check("status complete or partial", ok_status)
        .check("anchor is every 7200 s", |v| {
            find_sr(v, &["coffee"], "coffee")
                .is_some_and(|s| anchor_kind(s) == "every" && amount_is(s, "s", 7_200.0))
        })
        .check("marks every 2 h of driving", |v| {
            find_sr(v, &["coffee"], "coffee").is_some_and(|s| {
                let t = anchor_values(v, s, "cum_s");
                t.len() >= 3
                    && t.iter()
                        .enumerate()
                        .all(|(i, x)| near(*x, 7_200.0 * (i as f64 + 1.0), 1.0))
            })
        })
        .check("at least 2 coffee stops found", at_least_stops(2)),
        Scenario::new(
            "every-miles",
            "Drive to Berlin and stop for fuel every 100 miles",
        )
        .check("status complete or partial", ok_status)
        .check("anchor is every 160 934 m", |v| {
            find_sr(v, &["fuel"], "fuel").is_some_and(|s| amount_is(s, "m", 160_934.4))
        })
        .check("at least 3 fuel stops", at_least_stops(3)),
        Scenario::new(
            "every-too-long",
            "Drive to Utrecht with a charger every 500 km",
        )
        .check("plan parsed", |v| {
            find_sr(v, &["ev_charging"], "charg").is_some()
        })
        .check("status partial (route shorter than interval)", |v| {
            status(v) == "partial"
        })
        .check("reported as UNSUPPORTED, nothing invented", |v| {
            features(v, "unresolved_stop")
                .iter()
                .any(|f| f["properties"]["result"] == "UNSUPPORTED")
                && features(v, "stop").is_empty()
        }),
        // ---- offsets ----
        Scenario::new(
            "offset-after-origin-time",
            "Drive to Hamburg and find a petrol station 40 minutes in",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("offset from origin, after, 2400 s", |v| {
            find_sr(v, &["fuel"], "petrol").is_some_and(|s| {
                anchor_kind(s) == "offset"
                    && s["anchor"]["from"] == "origin"
                    && s["anchor"]["direction"] == "after"
                    && amount_is(s, "s", 2_400.0)
            })
        })
        .check("anchor at 2400 s of driving", |v| {
            features(v, "stop").first().is_some_and(|s| {
                near(
                    s["properties"]["anchor"]["cum_s"].as_f64().unwrap_or(0.0),
                    2_400.0,
                    1.0,
                )
            })
        }),
        Scenario::new(
            "offset-after-origin-km",
            "Driving to Brussels, find a restaurant after 100 km",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("offset from origin, after, 100 000 m", |v| {
            find_sr(v, &["restaurant", "fast_food"], "restaurant")
                .is_some_and(|s| s["anchor"]["from"] == "origin" && amount_is(s, "m", 100_000.0))
        })
        .check("anchor at 100 km", |v| {
            features(v, "stop").first().is_some_and(|s| {
                near(
                    s["properties"]["anchor"]["cum_m"].as_f64().unwrap_or(0.0),
                    100_000.0,
                    1.0,
                )
            })
        }),
        Scenario::new(
            "offset-before-destination-time-brand",
            "Drive to Paris and stop at a McDonald's 30 minutes before we arrive",
        )
        .at(UTRECHT)
        .check("status complete", |v| status(v) == "complete")
        .check("offset before destination, 1800 s", |v| {
            find_sr(v, &["fast_food"], "mcdonald").is_some_and(|s| {
                s["anchor"]["from"] == "destination"
                    && s["anchor"]["direction"] == "before"
                    && amount_is(s, "s", 1_800.0)
            })
        })
        .check("brand McDonald's in plan", |v| {
            find_sr(v, &["fast_food"], "mcdonald").is_some_and(|s| {
                s["what"]["brand"]
                    .as_str()
                    .is_some_and(|b| b.to_lowercase().contains("mcdonald"))
            })
        })
        .check("found place carries the McDonald's brand", |v| {
            features(v, "stop").iter().any(|s| {
                s["properties"]["pick"]["brand"]
                    .as_str()
                    .is_some_and(|b| b.to_lowercase().contains("mcdonald"))
            })
        }),
        Scenario::new(
            "offset-before-destination-km",
            "Going to Cologne, find a rest area 20 km before Cologne",
        )
        .check("status complete or partial", ok_status)
        .check("offset before destination, 20 000 m", |v| {
            find_sr(v, &["rest_area"], "rest").is_some_and(|s| {
                s["anchor"]["from"] == "destination" && amount_is(s, "m", 20_000.0)
            })
        }),
        // ---- fractions ----
        Scenario::new(
            "fraction-half-named-origin",
            "From Amsterdam to Frankfurt, lunch halfway",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("fraction 0.5", |v| {
            find_sr(v, &["restaurant", "fast_food"], "lunch").is_some_and(|s| {
                anchor_kind(s) == "fraction"
                    && near(s["anchor"]["value"].as_f64().unwrap_or(0.0), 0.5, 1e-9)
            })
        })
        .check("route plausible Amsterdam-Frankfurt (380-500 km)", |v| {
            (380_000.0..500_000.0).contains(&route_total_m(v))
        }),
        Scenario::new(
            "fraction-third",
            "Drive to Berlin, coffee a third of the way",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("fraction about 1/3", |v| {
            find_sr(v, &["coffee"], "coffee").is_some_and(|s| {
                anchor_kind(s) == "fraction"
                    && near(
                        s["anchor"]["value"].as_f64().unwrap_or(0.0),
                        1.0 / 3.0,
                        0.01,
                    )
            })
        }),
        // ---- waypoints ----
        Scenario::new(
            "waypoint-offset",
            "Drive to Berlin via Hannover, and find a supermarket 30 minutes after Hannover",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("one waypoint", |v| {
            v["plan"]["waypoints"]
                .as_array()
                .is_some_and(|w| w.len() == 1)
        })
        .check("offset from waypoint 0, after, 1800 s", |v| {
            find_sr(v, &["supermarket"], "supermarket").is_some_and(|s| {
                s["anchor"]["from"] == json!({"waypoint": 0}) && amount_is(s, "s", 1_800.0)
            })
        }),
        // ---- context ----
        Scenario::new("saved-place", "Drive home with a coffee halfway")
            .saved("home", UTRECHT)
            .check("status complete", |v| status(v) == "complete")
            .check("destination is the saved place", |v| {
                v["plan"]["destination"]["kind"] == "coords"
                    && v["plan"]["destination"]["label"] == "home"
            })
            .check("route plausible Amsterdam-Utrecht (20-70 km)", |v| {
                (20_000.0..70_000.0).contains(&route_total_m(v))
            }),
        Scenario::new("active-route-no-destination", "Coffee in about 20 minutes")
            .driving_to(BERLIN)
            .check("status complete", |v| status(v) == "complete")
            .check("destination from active route", |v| {
                v["plan"]["destination"]["label"] == "active_destination"
            })
            .check("offset after origin, 1200 s", |v| {
                find_sr(v, &["coffee"], "coffee")
                    .is_some_and(|s| s["anchor"]["from"] == "origin" && amount_is(s, "s", 1_200.0))
            }),
        // ---- what / filters ----
        Scenario::new(
            "free-text-place",
            "Drive to Rotterdam and find a bakery halfway",
        )
        .check("status complete or partial", ok_status)
        .check("bakery as free text", |v| find_sr(v, &[], "bak").is_some()),
        Scenario::new(
            "ev-connector-and-power",
            "Drive to Berlin with a 150 kW CCS2 charger halfway",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("ccs2 connector and 150 kW", |v| {
            find_sr(v, &["ev_charging"], "charg").is_some_and(|s| {
                s["what"]["ev"]["min_power_kw"].as_f64() == Some(150.0)
                    && s["what"]["ev"]["connector_types"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|x| x == "ccs2"))
            })
        }),
        Scenario::new(
            "max-detour",
            "Drive to Hamburg, coffee halfway, but no more than 5 minutes off the route",
        )
        .check("status complete or partial", ok_status)
        .check("max detour 300 s in plan", |v| {
            v["plan"]["max_detour_s"].as_u64() == Some(300)
        }),
        Scenario::new(
            "top-n-options",
            "Drive to Berlin and give me 3 options for lunch halfway",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("top_n 3", |v| {
            find_sr(v, &["restaurant", "fast_food"], "lunch").is_some_and(|s| s["top_n"] == 3)
        })
        .check("alternatives returned", |v| {
            features(v, "stop").iter().any(|s| {
                s["properties"]["alternatives"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
            })
        }),
        Scenario::new(
            "avoid-tolls",
            "Drive to Paris avoiding tolls, coffee halfway",
        )
        .check("status complete", |v| status(v) == "complete")
        .check("avoid tolls", |v| {
            v["plan"]["avoid"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "tolls"))
        }),
        Scenario::new(
            "multi-stop",
            "Drive to Berlin: fuel after 1 hour, lunch halfway, and a coffee 20 km before Berlin",
        )
        .check("status complete or partial", ok_status)
        .check("three stop requests", |v| stop_requests(v).len() == 3)
        .check("three distinct anchors", |v| {
            let a: HashSet<String> = stop_requests(v)
                .iter()
                .map(|s| format!("{}:{}", anchor_kind(s), s["anchor"]["from"]))
                .collect();
            a.len() == 3
        })
        .check("at least 2 stops found", at_least_stops(2)),
        // ---- developer settings ----
        Scenario::new(
            "settings-strict-2km",
            "Drive to Berlin with a coffee every 150 km",
        )
        .settings(Defaults {
            anchor_radius_m: 2_000,
            strict_radius: true,
            ..Default::default()
        })
        .check("status complete or partial", ok_status)
        .check("settings echoed (2000 m, strict)", |v| {
            v["search"]["anchor_radius_m"] == 2_000 && v["search"]["strict_radius"] == true
        })
        .check("every found stop within 2 km, none flagged", |v| {
            features(v, "stop").iter().all(|s| {
                s["properties"]["pick"]["distance_from_anchor_m"]
                    .as_f64()
                    .unwrap_or(f64::MAX)
                    <= 2_000.0
                    && s["properties"]["beyond_radius"] == false
            })
        }),
        Scenario::new("settings-limit-and-span", "Drive to Hamburg, fuel halfway")
            .settings(Defaults {
                search_limit: 5,
                search_span_m: Some(10_000),
                ..Default::default()
            })
            .check("status complete", |v| status(v) == "complete")
            .check("settings echoed (limit 5, span 10 000 m)", |v| {
                v["search"]["search_limit"] == 5 && v["search"]["search_span_m"] == 10_000
            })
            .check("search window is 2 x 10 km of route", |v| {
                features(v, "stop").iter().all(|s| {
                    let w = &s["properties"]["window"];
                    w["end_m"].as_f64().unwrap_or(0.0) - w["start_m"].as_f64().unwrap_or(0.0)
                        <= 20_001.0
                })
            }),
        Scenario::new(
            "settings-query-overrides-default",
            "Drive to Berlin, give me 2 options for coffee halfway, at most 3 minutes off route",
        )
        .settings(Defaults {
            max_detour_s: 900,
            top_n: 1,
            ..Default::default()
        })
        .check("status complete or partial", ok_status)
        .check("query's 3 min beats developer's 15 min", |v| {
            v["plan"]["max_detour_s"].as_u64() == Some(180)
        })
        .check("query's 2 options beat developer's 1", |v| {
            find_sr(v, &["coffee"], "coffee").is_some_and(|s| s["top_n"] == 2)
        }),
        Scenario::new(
            "settings-invalid-rejected",
            "Drive to Berlin, coffee halfway",
        )
        .settings(Defaults {
            anchor_radius_m: 10,
            ..Default::default()
        })
        .expect_invalid(),
        // ---- expected failures ----
        Scenario::new("fail-no-destination", "Coffee in 20 minutes")
            .check("status failed", |v| status(v) == "failed")
            .check("error INVALID_PLAN", |v| {
                v["error"]["code"] == "INVALID_PLAN"
            })
            .check("no map calls made", |v| v["usage"]["map_calls"] == 0),
        Scenario::new("fail-nonsense", "asdf qwerty zxcv")
            .check("status failed", |v| status(v) == "failed")
            .check("no map calls made", |v| v["usage"]["map_calls"] == 0),
    ]
}

fn context_for(sc: &Scenario) -> TripContext {
    TripContext {
        current_location: Some(Coords {
            lat: sc.at.0,
            lng: sc.at.1,
        }),
        active_destination: sc.active_destination.map(|(lat, lng)| Coords { lat, lng }),
        saved_places: sc
            .saved
            .iter()
            .map(|(n, (lat, lng))| SavedPlace {
                name: (*n).into(),
                location: Coords {
                    lat: *lat,
                    lng: *lng,
                },
            })
            .collect(),
        vehicle: sc.ev.then(|| Vehicle {
            ev: Some(EvFilter {
                min_power_kw: None,
                connector_types: vec![Connector::Ccs2],
            }),
        }),
        locale: Some("en".into()),
        defaults: sc.defaults,
        ..TripContext::default()
    }
}

fn print_summary(v: &Value) {
    let stops = features(v, "stop");
    let beyond = stops
        .iter()
        .filter(|s| s["properties"]["beyond_radius"] == true)
        .count();
    let max_dist = stops
        .iter()
        .filter_map(|s| s["properties"]["pick"]["distance_from_anchor_m"].as_f64())
        .fold(0.0_f64, f64::max);
    let err = v["error"]["code"]
        .as_str()
        .map(|c| {
            format!(
                " | error {c}: {}",
                v["error"]["message"].as_str().unwrap_or("")
            )
        })
        .unwrap_or_default();
    println!(
        "     status {} | stops {} ({} beyond radius, max {:.1} km from anchor) | unresolved {} | route {:.0} km | calls {} LLM, {} map{err}",
        status(v),
        stops.len(),
        beyond,
        max_dist / 1000.0,
        features(v, "unresolved_stop").len(),
        route_total_m(v) / 1000.0,
        v["usage"]["llm_calls"],
        v["usage"]["map_calls"],
    );
}

fn main() -> ExitCode {
    let env = load_dotenv();
    let filter = std::env::args().nth(1).unwrap_or_default();
    let llm = match llm_config(&env) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let (provider, map) = match map_provider(&env) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let Some(llm_key) = setting(&env, "LLM_API_KEY") else {
        eprintln!("LLM_API_KEY must be set (environment or .env)");
        return ExitCode::from(2);
    };
    let engine = Engine::new(EngineConfig::new(llm, provider));
    let keys = Keys { llm: llm_key, map };

    let (mut passed, mut failed, mut ran) = (0, 0, 0);
    let mut failed_names = Vec::new();
    for sc in scenarios().into_iter().filter(|s| s.name.contains(&filter)) {
        ran += 1;
        let req = TripRequest {
            query: sc.query.into(),
            context: context_for(&sc),
            request_id: Some(sc.name.into()),
        };
        println!("\n=== {}  \"{}\"", sc.name, sc.query);
        let result = drive(&engine, req, &keys);
        if sc.expect_invalid {
            let ok = matches!(&result, Err(e) if e.to_string().contains("invalid request"));
            println!(
                "{} invalid settings rejected before any call ({})",
                if ok { "PASS" } else { "FAIL" },
                result
                    .as_ref()
                    .err()
                    .map_or_else(|| "accepted".to_string(), ToString::to_string)
            );
            if ok {
                passed += 1
            } else {
                failed += 1;
                failed_names.push(sc.name)
            }
            continue;
        }
        let v: Value = match result {
            Ok(g) => serde_json::from_str(&g).unwrap_or_default(),
            Err(e) => {
                println!("FAIL engine error: {e}");
                failed += sc.checks.len();
                failed_names.push(sc.name);
                continue;
            }
        };
        print_summary(&v);
        let mut checks = sc.checks;
        checks.push(("within call budget", Box::new(budget_ok)));
        if status(&v) != "failed" {
            checks.push((
                "found stops within detour limit",
                Box::new(detours_within_limit),
            ));
            checks.push((
                "found stops within 5 km of anchor, or flagged beyond_radius",
                Box::new(within_radius_or_flagged),
            ));
        }
        let mut sc_failed = false;
        for (what, f) in &checks {
            let ok = f(&v);
            println!("{} {what}", if ok { "PASS" } else { "FAIL" });
            if ok {
                passed += 1;
            } else {
                failed += 1;
                sc_failed = true;
            }
        }
        if sc_failed {
            failed_names.push(sc.name);
        }
    }
    println!("\n===== {ran} scenarios, {passed} checks passed, {failed} failed =====");
    if !failed_names.is_empty() {
        println!("failing scenarios: {}", failed_names.join(", "));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
