//! Score a parser model against the held-out test set.
//!
//! ```text
//! # start a local OpenAI-compatible server first, e.g. mlx_lm.server --model <model> --adapter-path <adapters>
//! EVAL_BASE_URL=http://localhost:8080/v1 EVAL_MODEL=<model> \
//!   cargo run -q -p waylingo-reqwest --example eval_parser -- [limit] [test.jsonl]
//! ```
//!
//! For each test row, sends the row's system and user messages unchanged, takes the
//! reply text, and compares it with the expected plan two ways:
//!
//! * exact: the reply parses to the same `SurfaceIntent` as the label;
//! * plan: the reply, after the parser's place-name fix-up (`ground_in_query`),
//!   compiles to the same canonical `Plan` as the label (unit spellings and field
//!   order no longer matter). This is what the engine does and the number that counts;
//! * raw: the same check without the fix-up, to show what the model alone gets right.
//!
//! Prints totals, a breakdown by region and anchor kind, latency, and the first few
//! mismatches (request text only; these sentences are synthetic, not provider data).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use waylingo_parse::ground_in_query;
use waylingo_plan::{Coords, SavedPlace, SurfaceAnchor, SurfaceIntent, TripContext, compile};

fn ctx_for(row: &Value) -> TripContext {
    TripContext {
        current_location: Some(Coords {
            lat: 40.0,
            lng: 5.0,
        }),
        active_destination: row["meta"]["active_route"]
            .as_bool()
            .unwrap_or(false)
            .then_some(Coords {
                lat: 41.0,
                lng: 6.0,
            }),
        saved_places: row["saved"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_str())
                    .map(|n| SavedPlace {
                        name: n.into(),
                        location: Coords {
                            lat: 40.0,
                            lng: 5.0,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ..TripContext::default()
    }
}

fn extract_json(text: &str) -> &str {
    // Drop a <think>...</think> block if a reasoning model emits one, then take the
    // outermost {...}.
    let t = text.rsplit("</think>").next().unwrap_or(text).trim();
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if b > a => &t[a..=b],
        _ => t,
    }
}

fn anchor_kinds(i: &SurfaceIntent) -> Vec<String> {
    i.stops
        .iter()
        .map(|s| match &s.anchor {
            SurfaceAnchor::Offset { from, .. } => {
                format!("offset:{}", from.split(':').next().unwrap_or(""))
            }
            SurfaceAnchor::Every { .. } => "every".into(),
            SurfaceAnchor::Fraction { .. } => "fraction".into(),
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    n: usize,
    json_ok: usize,
    exact: usize,
    plan: usize,
    raw: usize,
}

impl Tally {
    fn add(&mut self, json_ok: bool, exact: bool, plan: bool, raw: bool) {
        self.n += 1;
        self.raw += usize::from(raw);
        self.json_ok += usize::from(json_ok);
        self.exact += usize::from(exact);
        self.plan += usize::from(plan);
    }
    fn line(&self, name: &str) -> String {
        let pct = |x: usize| {
            if self.n == 0 {
                0.0
            } else {
                100.0 * x as f64 / self.n as f64
            }
        };
        format!(
            "{name:28} n={:5}  valid JSON {:5.1}%  exact {:5.1}%  same plan {:5.1}%  raw {:5.1}%",
            self.n,
            pct(self.json_ok),
            pct(self.exact),
            pct(self.plan),
            pct(self.raw)
        )
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let limit: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(1000);
    let path = args
        .next()
        .unwrap_or_else(|| "finetune/data/test.jsonl".into());
    let base = std::env::var("EVAL_BASE_URL").unwrap_or_else(|_| "http://localhost:8080/v1".into());
    let model = std::env::var("EVAL_MODEL").unwrap_or_else(|_| "default".into());
    let key = std::env::var("EVAL_API_KEY").unwrap_or_default();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("http client");

    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let rows: Vec<Value> = text
        .lines()
        .take(limit)
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    println!(
        "evaluating {} rows from {path} against {base} (model {model})",
        rows.len()
    );

    let mut total = Tally::default();
    let mut by: BTreeMap<String, Tally> = BTreeMap::new();
    let mut latencies = Vec::new();
    let mut misses = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let msgs = &row["messages"];
        let expected: SurfaceIntent =
            serde_json::from_str(msgs[2]["content"].as_str().unwrap_or("{}"))
                .expect("label parses");
        let body = json!({ "model": model, "messages": [msgs[0], msgs[1]], "temperature": 0, "max_tokens": 400 });
        let mut req = client
            .post(format!("{}/chat/completions", base.trim_end_matches('/')))
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&body).unwrap_or_default());
        if !key.is_empty() {
            req = req.bearer_auth(&key);
        }
        let t0 = Instant::now();
        let reply = req
            .send()
            .ok()
            .and_then(|r| r.bytes().ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_default();
        latencies.push(t0.elapsed().as_secs_f64());

        let request = msgs[1]["content"]
            .as_str()
            .unwrap_or("")
            .rsplit("Request: ")
            .next()
            .unwrap_or("")
            .to_string();
        let raw: Option<SurfaceIntent> = serde_json::from_str(extract_json(&reply)).ok();
        let got = raw.clone().map(|mut g| {
            ground_in_query(&mut g, &request);
            g
        });
        let ctx = ctx_for(row);
        let want = compile(&expected, &ctx);
        let same_plan = |g: &Option<SurfaceIntent>| match (g, &want) {
            (Some(g), Ok(want)) => compile(g, &ctx).is_ok_and(|p| &p == want),
            // Negative examples: the right answer is something that does not compile.
            (Some(g), Err(_)) => compile(g, &ctx).is_err(),
            (None, Err(_)) => true,
            (None, Ok(_)) => false,
        };
        let exact = got.as_ref() == Some(&expected);
        let plan = same_plan(&got);
        let raw_plan = same_plan(&raw);
        total.add(got.is_some(), exact, plan, raw_plan);
        let region = row["meta"]["region"].as_str().unwrap_or("?").to_string();
        by.entry(format!("region {region}"))
            .or_default()
            .add(got.is_some(), exact, plan, raw_plan);
        for k in anchor_kinds(&expected) {
            by.entry(format!("anchor {k}"))
                .or_default()
                .add(got.is_some(), exact, plan, raw_plan);
        }
        by.entry(format!("stops {}", expected.stops.len()))
            .or_default()
            .add(got.is_some(), exact, plan, raw_plan);
        if !plan && misses.len() < 15 {
            let got_text: String = got
                .as_ref()
                .and_then(|g| serde_json::to_string(g).ok())
                .unwrap_or_else(|| extract_json(&reply).to_string())
                .chars()
                .take(400)
                .collect();
            misses.push((
                request.clone(),
                msgs[2]["content"].as_str().unwrap_or("").to_string(),
                got_text,
            ));
        }
        if (i + 1) % 100 == 0 {
            println!(
                "  {}/{}  same plan so far {:.1}%",
                i + 1,
                rows.len(),
                100.0 * total.plan as f64 / total.n as f64
            );
        }
    }
    latencies.sort_by(f64::total_cmp);
    let p = |q: f64| {
        latencies
            .get(((latencies.len() as f64 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!("\n{}", total.line("TOTAL"));
    for (k, t) in &by {
        println!("{}", t.line(k));
    }
    println!(
        "latency: p50 {:.2}s  p95 {:.2}s  max {:.2}s",
        p(0.5),
        p(0.95),
        p(1.0)
    );
    if !misses.is_empty() {
        println!("\nfirst mismatches:");
        for (q, want, got) in &misses {
            println!("  request: {q}\n    want: {want}\n    got:  {got}");
        }
    }
}
