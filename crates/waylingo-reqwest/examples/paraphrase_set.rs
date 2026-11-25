//! Build a "real wording" test set: a large LLM rewrites held-out requests the way
//! people actually type, and the original label is kept.
//!
//! ```text
//! cargo run -q -p waylingo-reqwest --example paraphrase_set -- [count] [in.jsonl] [out.jsonl]
//! # default: 300 rows from finetune/data2/test_unseen.jsonl into finetune/data2/test_real.jsonl
//! ```
//!
//! Uses the LLM from `.env` (`LLM_PROVIDER`, `LLM_API_KEY`, `LLM_MODEL`, ...). Only the
//! synthetic request sentence is sent; no map provider data is involved.
//!
//! A rewrite is kept only if every place, brand and free-text name from the label is
//! still in it character for character, and every number from the label is still in
//! it as digits. Otherwise the row is dropped, so the label stays correct. These rows
//! are for evaluation only and must never be used for training.

#[allow(dead_code)]
#[path = "common/settings.rs"]
mod settings;

use std::time::Duration;

use serde_json::{Value, json};
use waylingo_plan::{SurfaceAnchor, SurfaceIntent};

const STYLES: [&str; 8] = [
    "casual and short, like a quick text message",
    "polite and a bit wordy",
    "like a spoken voice command, no punctuation",
    "in a hurry, with abbreviations such as mins, kms, pls",
    "as a question to the assistant",
    "with the stops mentioned before the destination",
    "chatty, with a little extra context about the trip",
    "terse, as few words as possible",
];

fn names(i: &SurfaceIntent) -> Vec<String> {
    i.origin
        .iter()
        .chain(i.destination.iter())
        .chain(i.waypoints.iter())
        .filter(|p| !p.starts_with("saved:"))
        .cloned()
        .chain(
            i.stops
                .iter()
                .flat_map(|s| s.what.brand.iter().chain(s.what.text.iter()).cloned()),
        )
        .collect()
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Numbers the rewrite must keep as digits (fractions like "halfway" have none).
fn numbers(i: &SurfaceIntent) -> Vec<String> {
    let mut out: Vec<String> = i.max_detour.iter().map(|q| fmt_num(q.value)).collect();
    for s in &i.stops {
        match &s.anchor {
            SurfaceAnchor::Offset { amount, .. } | SurfaceAnchor::Every { amount, .. } => {
                out.push(fmt_num(amount.value));
            }
            SurfaceAnchor::Fraction { .. } => {}
        }
        out.extend(s.top_n.map(|k| k.to_string()));
        out.extend(s.what.min_power_kw.map(fmt_num));
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let input = args
        .next()
        .unwrap_or_else(|| "finetune/data2/test_unseen.jsonl".into());
    let output = args
        .next()
        .unwrap_or_else(|| "finetune/data2/test_real.jsonl".into());

    let env = settings::load_dotenv();
    let cfg = settings::llm_config(&env).unwrap_or_else(|e| panic!("{e}"));
    let key = settings::setting(&env, "LLM_API_KEY").expect("set LLM_API_KEY");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("http client");

    let text = std::fs::read_to_string(&input).unwrap_or_else(|e| panic!("{input}: {e}"));
    let mut kept = Vec::new();
    let (mut tried, mut dropped) = (0, 0);
    for (n, line) in text.lines().enumerate() {
        if kept.len() >= count {
            break;
        }
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let msgs = &row["messages"];
        let user = msgs[1]["content"].as_str().unwrap_or("");
        let Some((head, request)) = user.rsplit_once("Request: ") else {
            continue;
        };
        let Ok(label) =
            serde_json::from_str::<SurfaceIntent>(msgs[2]["content"].as_str().unwrap_or(""))
        else {
            continue;
        };
        if label.error.is_some() {
            continue;
        }
        tried += 1;
        let keep_names = names(&label);
        let keep_nums = numbers(&label);
        let style = STYLES[n % STYLES.len()];
        let system = format!(
            "Rewrite a driver's trip request the way a real person might type or say it to a car \
             assistant. Style: {style}. Keep exactly the same meaning: the same destination, \
             start and via places, the same stops in the same order, the same distances, \
             times and counts. Copy these names exactly, character for character, including \
             case: {names}. Write every number as digits. Do not add or remove stops, places \
             or conditions. Reply with the rewritten request only.",
            names = if keep_names.is_empty() {
                "(none)".to_string()
            } else {
                keep_names
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        let body = json!({
            "model": cfg.model,
            "max_tokens": 200,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": request },
            ],
        });
        let reply = client
            .post(format!(
                "{}/chat/completions",
                cfg.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&key)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&body).unwrap_or_default())
            .send()
            .ok()
            .and_then(|r| r.bytes().ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(str::to_string)
            })
            .map(|s| s.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        let ok = !reply.is_empty()
            && reply.len() < 400
            && keep_names.iter().all(|s| reply.contains(s.as_str()))
            && keep_nums.iter().all(|d| reply.contains(d.as_str()));
        if !ok {
            dropped += 1;
            continue;
        }
        let mut out = row.clone();
        out["messages"][1]["content"] = Value::String(format!("{head}Request: {reply}"));
        out["meta"]["real_wording"] = Value::Bool(true);
        out["meta"]["original"] = Value::String(request.to_string());
        if kept.len() < 8 {
            println!("  {request}\n    -> {reply}");
        }
        kept.push(out.to_string());
        if kept.len() % 50 == 0 {
            println!("  kept {} (tried {tried}, dropped {dropped})", kept.len());
        }
    }
    std::fs::write(&output, kept.join("\n") + "\n").expect("write output");
    println!(
        "wrote {} rows to {output} (tried {tried}, dropped {dropped})",
        kept.len()
    );
}
