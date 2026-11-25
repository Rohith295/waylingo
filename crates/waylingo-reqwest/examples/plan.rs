//! Plan a trip from the command line: natural language in, GeoJSON file out.
//!
//! ```text
//! cargo run -p waylingo-reqwest --example plan -- "Drive to Berlin, a fast charger every 200 km and a McDonald's 15 minutes before we arrive"
//! cargo run -p waylingo-reqwest --example plan -- --lat 52.3702 --lng 4.8952 --ev --out trip.geojson "coffee in 20 minutes"
//! cargo run -p waylingo-reqwest --example plan -- --radius-m 2000 --limit 10 --strict "Drive to Berlin, lunch halfway"
//! ```
//!
//! Search settings (developer defaults; a value said in the query wins for that query):
//! `--radius-m` (default 5000), `--span-m` (default 3 x radius), `--limit` (default 20,
//! capped by the provider), `--max-detour-s` (default 600), `--top-n` (default 1),
//! `--strict` (never return places beyond the radius).
//!
//! ```text
//! ```
//!
//! Settings come from the environment, or from a `.env` file in the current
//! directory (never committed):
//!
//! * `TOMTOM_API_KEY` (required)
//! * `LLM_API_KEY` (required)
//! * `LLM_PROVIDER`: `anthropic` (default), `openai`, or `ollama`
//! * `LLM_MODEL`, `LLM_BASE_URL`, `LLM_MODE` (`json_schema`, `tool_call`, `prompt_json`): optional overrides
//!
//! The GeoJSON is written to a file. The terminal only shows a summary (status,
//! counts, distance) and never place names, so provider data does not end up in a
//! terminal log an AI assistant might read. Keys are never printed.

use std::process::ExitCode;

use waylingo::{
    Connector, Coords, Defaults, Engine, EngineConfig, EvFilter, TripContext, TripRequest, Vehicle,
};
use waylingo_reqwest::{Keys, drive};

#[path = "common/settings.rs"]
mod settings;
use settings::{llm_config, load_dotenv, map_provider, setting};

struct Args {
    query: String,
    lat: f64,
    lng: f64,
    ev: bool,
    out: String,
    defaults: Defaults,
}

fn num<T: std::str::FromStr>(
    it: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, String> {
    it.next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{flag} needs a number"))
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        query: String::new(),
        lat: 52.3702,
        lng: 4.8952,
        ev: false,
        out: "waylingo-output.geojson".into(),
        defaults: Defaults::default(),
    };
    let mut words = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--lat" => {
                a.lat = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--lat needs a number")?
            }
            "--lng" => {
                a.lng = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--lng needs a number")?
            }
            "--out" => a.out = it.next().ok_or("--out needs a path")?,
            "--ev" => a.ev = true,
            "--radius-m" => a.defaults.anchor_radius_m = num(&mut it, "--radius-m")?,
            "--span-m" => a.defaults.search_span_m = Some(num(&mut it, "--span-m")?),
            "--limit" => a.defaults.search_limit = num(&mut it, "--limit")?,
            "--max-detour-s" => a.defaults.max_detour_s = num(&mut it, "--max-detour-s")?,
            "--top-n" => a.defaults.top_n = num(&mut it, "--top-n")?,
            "--strict" => a.defaults.strict_radius = true,
            _ => words.push(x),
        }
    }
    a.query = words.join(" ");
    if a.query.trim().is_empty() {
        return Err("give a query, e.g. \"coffee in 20 minutes\"".into());
    }
    Ok(a)
}

fn summarize(geojson: &str, out: &str) {
    let v: serde_json::Value = serde_json::from_str(geojson).unwrap_or_default();
    let feats = v["features"].as_array().cloned().unwrap_or_default();
    let of_kind = |k: &str| {
        feats
            .iter()
            .filter(|f| f["properties"]["kind"] == k)
            .cloned()
            .collect::<Vec<_>>()
    };
    println!("status:      {}", v["status"].as_str().unwrap_or("?"));
    if let Some(e) = v["error"].as_object() {
        println!(
            "error:       {} - {}",
            e["code"].as_str().unwrap_or("?"),
            e["message"].as_str().unwrap_or("")
        );
    }
    if let Some(r) = of_kind("route").first() {
        let p = &r["properties"];
        println!(
            "route:       {:.0} km, {:.1} h, {} points, includes stops: {}",
            p["total_m"].as_f64().unwrap_or(0.0) / 1000.0,
            p["total_s"].as_f64().unwrap_or(0.0) / 3600.0,
            r["geometry"]["coordinates"].as_array().map_or(0, Vec::len),
            p["includes_stops"]
        );
    }
    println!(
        "understood:  {} stop request(s)",
        v["plan"]["stop_requests"].as_array().map_or(0, Vec::len)
    );
    let stops = of_kind("stop");
    println!("stops found: {}", stops.len());
    for f in &stops {
        let p = &f["properties"];
        println!(
            "  - {} #{}: detour {} s, anchor at {:.0} km, {} alternative(s)",
            p["stop_request_id"].as_str().unwrap_or("?"),
            p["occurrence"],
            p["pick"]["detour_s"],
            p["anchor"]["cum_m"].as_f64().unwrap_or(0.0) / 1000.0,
            p["alternatives"].as_array().map_or(0, Vec::len)
        );
    }
    let unresolved = of_kind("unresolved_stop");
    println!("unresolved:  {}", unresolved.len());
    for f in &unresolved {
        let p = &f["properties"];
        println!(
            "  - {} #{}: {} {}",
            p["stop_request_id"].as_str().unwrap_or("?"),
            p["occurrence"],
            p["result"].as_str().unwrap_or("?"),
            p["detail"].as_str().unwrap_or("")
        );
    }
    for w in v["warnings"].as_array().into_iter().flatten() {
        println!("warning:     {}", w.as_str().unwrap_or(""));
    }
    let s = &v["search"];
    println!(
        "search:      radius {} m, span {} m, limit {}, strict {}",
        s["anchor_radius_m"], s["search_span_m"], s["search_limit"], s["strict_radius"]
    );
    println!(
        "calls:       {} LLM, {} map",
        v["usage"]["llm_calls"], v["usage"]["map_calls"]
    );
    println!("written to:  {out}  (open it on a map to see names and places)");
}

fn main() -> ExitCode {
    let env = load_dotenv();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
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
    let vehicle = args.ev.then(|| Vehicle {
        ev: Some(EvFilter {
            min_power_kw: None,
            connector_types: vec![Connector::Ccs2],
        }),
    });
    let request = TripRequest {
        query: args.query,
        context: TripContext {
            current_location: Some(Coords {
                lat: args.lat,
                lng: args.lng,
            }),
            vehicle,
            locale: Some("en".into()),
            defaults: args.defaults,
            ..TripContext::default()
        },
        request_id: None,
    };
    let keys = Keys { llm: llm_key, map };
    match drive(&engine, request, &keys) {
        Ok(geojson) => {
            if let Err(e) = std::fs::write(&args.out, &geojson) {
                eprintln!("could not write {}: {e}", args.out);
                return ExitCode::FAILURE;
            }
            summarize(&geojson, &args.out);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
